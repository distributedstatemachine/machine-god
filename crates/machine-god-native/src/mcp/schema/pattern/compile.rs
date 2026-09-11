use super::{Instruction, Node, Op, PatternError, Result};

struct Fragment {
    start: usize,
    pending: Vec<(usize, bool)>,
}
struct Compiler<'a> {
    nodes: &'a [Node],
    instructions: Vec<Instruction>,
    limit: usize,
}
pub(super) fn compile(
    nodes: &[Node],
    root: usize,
    limit: usize,
) -> Result<(Vec<Instruction>, usize)> {
    let mut compiler = Compiler {
        nodes,
        instructions: Vec::new(),
        limit,
    };
    let fragment = compiler.node(root)?;
    let end = compiler.add(Op::Accept, 0, 0)?;
    compiler.patch(&fragment.pending, end);
    Ok((compiler.instructions, fragment.start))
}
impl Compiler<'_> {
    fn add(&mut self, op: Op, next: usize, alternate: usize) -> Result<usize> {
        if self.instructions.len() >= self.limit {
            return Err(PatternError::Limit);
        }
        let index = self.instructions.len();
        self.instructions.push(Instruction {
            op,
            next,
            alternate,
        });
        Ok(index)
    }
    fn single(&mut self, op: Op) -> Result<Fragment> {
        let start = self.add(op, 0, 0)?;
        Ok(Fragment {
            start,
            pending: vec![(start, false)],
        })
    }
    fn patch(&mut self, pending: &[(usize, bool)], target: usize) {
        for &(index, alternate) in pending {
            if alternate {
                self.instructions[index].alternate = target;
            } else {
                self.instructions[index].next = target;
            }
        }
    }
    fn concatenate(&mut self, left: &Fragment, right: Fragment) -> Fragment {
        self.patch(&left.pending, right.start);
        Fragment {
            start: left.start,
            pending: right.pending,
        }
    }
    fn node(&mut self, index: usize) -> Result<Fragment> {
        match self.nodes[index] {
            Node::Empty => self.single(Op::Epsilon),
            Node::Literal(value) => self.single(Op::Literal(value)),
            Node::Any => self.single(Op::Any),
            Node::Class(value) => self.single(Op::Class(value)),
            Node::Start => self.single(Op::Start),
            Node::End => self.single(Op::End),
            Node::Concat(left, right) => {
                let left = self.node(left)?;
                let right = self.node(right)?;
                Ok(self.concatenate(&left, right))
            }
            Node::Alternate(left, right) => {
                let mut left = self.node(left)?;
                let right = self.node(right)?;
                let start = self.add(Op::Split, left.start, right.start)?;
                left.pending.extend(right.pending);
                Ok(Fragment {
                    start,
                    pending: left.pending,
                })
            }
            Node::Repeat { child, min, max } => {
                let mut result = self.single(Op::Epsilon)?;
                for _ in 0..min {
                    let next = self.node(child)?;
                    result = self.concatenate(&result, next);
                }
                if let Some(maximum) = max {
                    for _ in min..maximum {
                        let mut next = self.node(child)?;
                        let start = self.add(Op::Split, next.start, 0)?;
                        next.pending.push((start, true));
                        result = self.concatenate(
                            &result,
                            Fragment {
                                start,
                                pending: next.pending,
                            },
                        );
                    }
                } else {
                    let next = self.node(child)?;
                    let start = self.add(Op::Split, next.start, 0)?;
                    self.patch(&next.pending, start);
                    result = self.concatenate(
                        &result,
                        Fragment {
                            start,
                            pending: vec![(start, true)],
                        },
                    );
                }
                Ok(result)
            }
        }
    }
}
