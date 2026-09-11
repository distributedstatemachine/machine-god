use super::{Class, McpSchemaLimits, Node, PatternError as Error, Result};

pub(super) fn parse(
    source: &str,
    limits: McpSchemaLimits,
) -> Result<(Vec<Node>, Vec<Class>, usize)> {
    let mut parser = Parser {
        source: source.chars().collect(),
        cursor: 0,
        depth: 0,
        nodes: Vec::new(),
        classes: Vec::new(),
        limits,
    };
    let root = parser.alternation(false)?;
    if parser.cursor != parser.source.len() {
        return Err(Error::Unsupported);
    }
    Ok((parser.nodes, parser.classes, root))
}
struct Parser {
    source: Vec<char>,
    cursor: usize,
    depth: usize,
    nodes: Vec<Node>,
    classes: Vec<Class>,
    limits: McpSchemaLimits,
}
impl Parser {
    fn peek(&self) -> Option<char> {
        self.source.get(self.cursor).copied()
    }
    fn take(&mut self, value: char) -> bool {
        if self.peek() != Some(value) {
            return false;
        }
        self.cursor += 1;
        true
    }
    fn next(&mut self) -> Result<char> {
        let value = self.peek().ok_or(Error::Unsupported)?;
        self.cursor += 1;
        Ok(value)
    }
    fn add(&mut self, node: Node) -> Result<usize> {
        if self.nodes.len() >= self.limits.max_pattern_states {
            return Err(Error::Limit);
        }
        let index = self.nodes.len();
        self.nodes.push(node);
        Ok(index)
    }
    fn balanced(&mut self, nodes: &[usize], alternate: bool) -> Result<usize> {
        match nodes {
            [] => self.add(Node::Empty),
            [node] => Ok(*node),
            _ => {
                let middle = nodes.len() / 2;
                let left = self.balanced(&nodes[..middle], alternate)?;
                let right = self.balanced(&nodes[middle..], alternate)?;
                self.add(if alternate {
                    Node::Alternate(left, right)
                } else {
                    Node::Concat(left, right)
                })
            }
        }
    }
    fn alternation(&mut self, grouped: bool) -> Result<usize> {
        let mut branches = vec![self.sequence(grouped)?];
        while self.take('|') {
            branches.push(self.sequence(grouped)?);
        }
        self.balanced(&branches, true)
    }
    fn sequence(&mut self, grouped: bool) -> Result<usize> {
        let mut nodes = Vec::new();
        while self
            .peek()
            .is_some_and(|value| value != '|' && !(grouped && value == ')'))
        {
            nodes.push(self.repetition()?);
        }
        self.balanced(&nodes, false)
    }
    fn repetition(&mut self) -> Result<usize> {
        let (child, quantifiable) = self.atom()?;
        let Some(operator @ ('*' | '+' | '?' | '{')) = self.peek() else {
            return Ok(child);
        };
        if !quantifiable {
            return Err(Error::Unsupported);
        }
        self.cursor += 1;
        let (min, max) = match operator {
            '*' => (0, None),
            '+' => (1, None),
            '?' => (0, Some(1)),
            _ => {
                let min = self.count()?;
                if self.take('}') {
                    (min, Some(min))
                } else {
                    if !self.take(',') {
                        return Err(Error::Unsupported);
                    }
                    let max = if self.take('}') {
                        None
                    } else {
                        let max = self.count()?;
                        if !self.take('}') {
                            return Err(Error::Unsupported);
                        }
                        Some(max)
                    };
                    (min, max)
                }
            }
        };
        if max.is_some_and(|maximum| maximum < min) {
            return Err(Error::Unsupported);
        }
        if min > self.limits.max_pattern_repeat
            || max.is_some_and(|maximum| maximum > self.limits.max_pattern_repeat)
        {
            return Err(Error::Limit);
        }
        self.take('?');
        if matches!(self.peek(), Some('*' | '+' | '?' | '{')) {
            return Err(Error::Unsupported);
        }
        self.add(Node::Repeat { child, min, max })
    }
    fn atom(&mut self) -> Result<(usize, bool)> {
        let character = self.next()?;
        let node = match character {
            '^' => return Ok((self.add(Node::Start)?, false)),
            '$' => return Ok((self.add(Node::End)?, false)),
            '.' => Node::Any,
            '(' => {
                if self.peek() == Some('?') {
                    return Err(Error::Unsupported);
                }
                if self.depth >= self.limits.max_depth {
                    return Err(Error::Limit);
                }
                self.depth += 1;
                let child = self.alternation(true)?;
                self.depth -= 1;
                if !self.take(')') {
                    return Err(Error::Unsupported);
                }
                return Ok((child, true));
            }
            '[' => return Ok((self.class()?, true)),
            '\\' => return Ok((self.escape_node()?, true)),
            ')' | ']' | '}' | '|' | '*' | '+' | '?' | '{' => return Err(Error::Unsupported),
            value => Node::Literal(value),
        };
        Ok((self.add(node)?, true))
    }
    fn class_node(&mut self, class: Class) -> Result<usize> {
        if self.classes.len() >= self.limits.max_pattern_states {
            return Err(Error::Limit);
        }
        let index = self.classes.len();
        self.classes.push(class);
        self.add(Node::Class(index))
    }
    fn escape_node(&mut self) -> Result<usize> {
        let escaped = self.next()?;
        if matches!(escaped, 'd' | 'D' | 'w' | 'W' | 's' | 'S') {
            let flags = match escaped.to_ascii_lowercase() {
                'd' => 1,
                'w' => 2,
                _ => 4,
            };
            return self.class_node(Class {
                flags,
                negated: escaped.is_ascii_uppercase(),
                ..Class::default()
            });
        }
        if matches!(escaped, 'p' | 'P') {
            if !self.take('{') {
                return Err(Error::Unsupported);
            }
            let start = self.cursor;
            while self.peek().is_some_and(|value| value != '}') {
                self.cursor += 1;
            }
            let name = self.source[start..self.cursor].iter().collect::<String>();
            if !self.take('}') || !matches!(name.as_str(), "Letter" | "L") {
                return Err(Error::Unsupported);
            }
            return self.class_node(Class {
                flags: 16,
                negated: escaped == 'P',
                ..Class::default()
            });
        }
        let value = self.escaped_literal(escaped, false)?;
        self.add(Node::Literal(value))
    }
    fn class(&mut self) -> Result<usize> {
        let mut class = Class {
            negated: self.take('^'),
            ..Class::default()
        };
        let mut first = true;
        while self.peek().is_some() {
            if !first && self.take(']') {
                return self.class_node(class);
            }
            let left = self.class_atom(&mut class)?;
            first = false;
            let range = self.peek() == Some('-')
                && self
                    .source
                    .get(self.cursor + 1)
                    .is_some_and(|value| *value != ']');
            let Some(left) = left else {
                if range {
                    return Err(Error::Unsupported);
                }
                continue;
            };
            let right = if range {
                self.cursor += 1;
                self.class_atom(&mut class)?.ok_or(Error::Unsupported)?
            } else {
                left
            };
            if right < left {
                return Err(Error::Unsupported);
            }
            class.ranges.push((left, right));
            if class.ranges.len() > self.limits.max_pattern_states {
                return Err(Error::Limit);
            }
        }
        Err(Error::Unsupported)
    }
    fn class_atom(&mut self, class: &mut Class) -> Result<Option<char>> {
        let value = self.next()?;
        if value != '\\' {
            return Ok(Some(value));
        }
        let escaped = self.next()?;
        match escaped {
            'd' => class.flags |= 1,
            'w' => class.flags |= 2,
            's' => class.flags |= 4,
            'S' => class.flags |= 8,
            _ => return self.escaped_literal(escaped, true).map(Some),
        }
        Ok(None)
    }
    fn escaped_literal(&mut self, escaped: char, class: bool) -> Result<char> {
        match escaped {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'f' => Ok('\u{C}'),
            'v' => Ok('\u{B}'),
            'x' => self.hex(2),
            'u' => {
                if !class && self.take('{') {
                    let start = self.cursor;
                    while self.peek().is_some_and(|value| value != '}') {
                        self.cursor += 1;
                    }
                    let count = self.cursor - start;
                    self.cursor = start;
                    if count == 0 || count > 6 {
                        return Err(Error::Unsupported);
                    }
                    let value = self.hex(count)?;
                    if !self.take('}') {
                        return Err(Error::Unsupported);
                    }
                    Ok(value)
                } else {
                    self.hex(4)
                }
            }
            '0' if !self.peek().is_some_and(|value| value.is_ascii_digit()) => Ok('\0'),
            '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
            | '/' => Ok(escaped),
            '-' if class => Ok('-'),
            _ => Err(Error::Unsupported),
        }
    }
    fn hex(&mut self, count: usize) -> Result<char> {
        let mut value = 0_u32;
        for _ in 0..count {
            value = value * 16 + self.next()?.to_digit(16).ok_or(Error::Unsupported)?;
        }
        char::from_u32(value).ok_or(Error::Unsupported)
    }
    fn count(&mut self) -> Result<usize> {
        let mut value = 0_usize;
        let start = self.cursor;
        while let Some(digit) = self
            .peek()
            .and_then(|value| value.to_digit(10))
            .filter(|_| self.peek().is_some_and(|value| value.is_ascii_digit()))
        {
            self.cursor += 1;
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::try_from(digit).ok()?))
                .ok_or(Error::Limit)?;
        }
        if self.cursor == start {
            return Err(Error::Unsupported);
        }
        Ok(value)
    }
}
