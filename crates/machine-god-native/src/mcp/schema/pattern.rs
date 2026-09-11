//! Pinned bounded Thompson evaluator, with explicit unsupported-grammar results.
use super::McpSchemaLimits;

mod compile;
mod parse;
mod unicode_letter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatternError {
    Unsupported,
    Limit,
}
type Result<T> = std::result::Result<T, PatternError>;

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Literal(char),
    Any,
    Class(usize),
    Start,
    End,
    Concat(usize, usize),
    Alternate(usize, usize),
    Repeat {
        child: usize,
        min: usize,
        max: Option<usize>,
    },
}
#[derive(Clone, Debug, Default)]
struct Class {
    ranges: Vec<(char, char)>,
    flags: u8,
    negated: bool,
}
impl Class {
    fn matches(&self, value: char) -> bool {
        let matched = self.flags & 1 != 0 && value.is_ascii_digit()
            || self.flags & 2 != 0 && (value.is_ascii_alphanumeric() || value == '_')
            || self.flags & 4 != 0 && whitespace(value)
            || self.flags & 8 != 0 && !whitespace(value)
            || self.flags & 16 != 0 && unicode_letter::contains(value)
            || self
                .ranges
                .iter()
                .any(|(first, last)| *first <= value && value <= *last);
        matched != self.negated
    }
}
fn whitespace(value: char) -> bool {
    matches!(u32::from(value), 0x0009..=0x000D | 0x0020 | 0x00A0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF)
}
#[derive(Clone, Copy, Debug)]
enum Op {
    Epsilon,
    Literal(char),
    Any,
    Class(usize),
    Split,
    Start,
    End,
    Accept,
}
#[derive(Clone, Copy, Debug)]
struct Instruction {
    op: Op,
    next: usize,
    alternate: usize,
}
pub(crate) struct Pattern {
    instructions: Vec<Instruction>,
    classes: Vec<Class>,
    start: usize,
}
impl Pattern {
    pub fn compile(source: &str, limits: McpSchemaLimits) -> Result<Self> {
        let (nodes, classes, root) = parse::parse(source, limits)?;
        let (instructions, start) = compile::compile(&nodes, root, limits.max_pattern_states)?;
        Ok(Self {
            instructions,
            classes,
            start,
        })
    }
    pub fn states(&self) -> usize {
        self.instructions.len()
    }
    pub fn retained_byte_charge(&self) -> super::Result<usize> {
        use super::accounting::{add, array};
        let mut bytes = array::<Instruction>(self.instructions.capacity())?;
        add(
            &mut bytes,
            array::<Class>(self.classes.capacity())?,
            usize::MAX,
        )?;
        for class in &self.classes {
            add(
                &mut bytes,
                array::<(char, char)>(class.ranges.capacity())?,
                usize::MAX,
            )?;
        }
        Ok(bytes)
    }
    pub fn matches(&self, text: &str, steps: &mut usize, limit: usize) -> Result<bool> {
        let mut current = States::new(self.instructions.len());
        let mut next = States::new(self.instructions.len());
        let mut work = Closure {
            visited: vec![0; self.instructions.len()],
            generation: 0,
            stack: Vec::with_capacity(self.instructions.len()),
            length: text.len(),
            steps,
            limit,
        };
        let mut cursor = 0;
        loop {
            self.closure(self.start, cursor, &mut current, &mut work)?;
            if current
                .active
                .iter()
                .any(|index| matches!(self.instructions[*index].op, Op::Accept))
            {
                return Ok(true);
            }
            let Some(value) = text[cursor..].chars().next() else {
                return Ok(false);
            };
            cursor += value.len_utf8();
            next.clear();
            for index in &current.active {
                let instruction = self.instructions[*index];
                let matched = match instruction.op {
                    Op::Literal(literal) => value == literal,
                    Op::Any => !matches!(value, '\n' | '\r' | '\u{2028}' | '\u{2029}'),
                    Op::Class(class) => self.classes[class].matches(value),
                    _ => false,
                };
                if matched {
                    consume(work.steps, work.limit)?;
                    self.closure(instruction.next, cursor, &mut next, &mut work)?;
                }
            }
            std::mem::swap(&mut current, &mut next);
        }
    }
    fn closure(
        &self,
        start: usize,
        cursor: usize,
        states: &mut States,
        work: &mut Closure<'_>,
    ) -> Result<()> {
        work.generation = work.generation.checked_add(1).ok_or(PatternError::Limit)?;
        work.stack.clear();
        work.stack.push(start);
        while let Some(index) = work.stack.pop() {
            if work.visited[index] == work.generation {
                continue;
            }
            work.visited[index] = work.generation;
            consume(work.steps, work.limit)?;
            let instruction = self.instructions[index];
            match instruction.op {
                Op::Epsilon => work.stack.push(instruction.next),
                Op::Split => {
                    work.stack.push(instruction.next);
                    work.stack.push(instruction.alternate);
                }
                Op::Start if cursor == 0 => work.stack.push(instruction.next),
                Op::End if cursor == work.length => work.stack.push(instruction.next),
                Op::Start | Op::End => {}
                _ => states.insert(index),
            }
        }
        Ok(())
    }
}
struct States {
    active: Vec<usize>,
    present: Vec<bool>,
}
impl States {
    fn new(length: usize) -> Self {
        Self {
            active: Vec::with_capacity(length),
            present: vec![false; length],
        }
    }
    fn insert(&mut self, index: usize) {
        if !self.present[index] {
            self.present[index] = true;
            self.active.push(index);
        }
    }
    fn clear(&mut self) {
        for index in self.active.drain(..) {
            self.present[index] = false;
        }
    }
}
struct Closure<'a> {
    visited: Vec<usize>,
    generation: usize,
    stack: Vec<usize>,
    length: usize,
    steps: &'a mut usize,
    limit: usize,
}
fn consume(steps: &mut usize, limit: usize) -> Result<()> {
    *steps = steps.checked_add(1).ok_or(PatternError::Limit)?;
    if *steps > limit {
        return Err(PatternError::Limit);
    }
    Ok(())
}
