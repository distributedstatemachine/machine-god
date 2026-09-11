use std::cmp::Ordering;

use super::{McpSchemaError, McpSchemaLimits, Result};

#[derive(Debug)]
pub(crate) struct Number {
    pub negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}
impl Number {
    /// Input has already passed the JSON parser; no rounded float enters here.
    pub fn parse(text: &str, limits: McpSchemaLimits, error: McpSchemaError) -> Result<Self> {
        if text.len() > limits.max_number_bytes {
            return Err(error);
        }
        let negative = text.starts_with('-');
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        let (mantissa, exponent_text) = unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, "0"), |parts| parts);
        let exponent_negative = exponent_text.starts_with('-');
        let mut explicit = 0_i64;
        for digit in exponent_text.trim_start_matches(['+', '-']).bytes() {
            explicit = explicit
                .checked_mul(10)
                .and_then(|value| value.checked_add(i64::from(digit - b'0')))
                .ok_or(error)?;
            if explicit > limits.max_number_exponent_abs {
                return Err(error);
            }
        }
        if exponent_negative {
            explicit = -explicit;
        }
        let fraction = mantissa.split_once('.').map_or(0, |(_, tail)| tail.len());
        let digits = mantissa
            .bytes()
            .filter(|byte| *byte != b'.')
            .collect::<Vec<_>>();
        let Some(first) = digits.iter().position(|digit| *digit != b'0') else {
            return Ok(Self {
                negative: false,
                digits: vec![b'0'],
                exponent: 0,
            });
        };
        let end = digits
            .iter()
            .rposition(|digit| *digit != b'0')
            .ok_or(error)?
            + 1;
        let exponent = explicit - i64::try_from(fraction).map_err(|_| error)?
            + i64::try_from(digits.len() - end).map_err(|_| error)?;
        if exponent.abs() > limits.max_number_exponent_abs {
            return Err(error);
        }
        Ok(Self {
            negative,
            digits: digits[first..end].to_vec(),
            exponent,
        })
    }
    pub fn zero(&self) -> bool {
        self.digits == [b'0']
    }
    pub fn integer(&self) -> bool {
        self.zero() || self.exponent >= 0
    }
    pub fn order(&self, other: &Self) -> Ordering {
        match (self.zero(), other.zero()) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if other.negative {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if self.negative {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            _ => {}
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let left_places = i64::try_from(self.digits.len()).unwrap_or(i64::MAX) + self.exponent;
        let right_places = i64::try_from(other.digits.len()).unwrap_or(i64::MAX) + other.exponent;
        let mut order = left_places.cmp(&right_places);
        if order == Ordering::Equal {
            for index in 0..self.digits.len().max(other.digits.len()) {
                order = self
                    .digits
                    .get(index)
                    .unwrap_or(&b'0')
                    .cmp(other.digits.get(index).unwrap_or(&b'0'));
                if order != Ordering::Equal {
                    break;
                }
            }
        }
        if self.negative {
            order.reverse()
        } else {
            order
        }
    }
    pub fn nonnegative_usize(&self) -> Option<usize> {
        if self.negative || !self.integer() {
            return None;
        }
        // Avoid spending a million iterations on an impossible usize conversion.
        if !self.zero() && self.exponent > 20 {
            return None;
        }
        let mut result = 0_usize;
        for digit in &self.digits {
            result = result
                .checked_mul(10)?
                .checked_add(usize::from(digit - b'0'))?;
        }
        for _ in 0..self.exponent {
            result = result.checked_mul(10)?;
        }
        Some(result)
    }
    pub fn multiple_of(&self, divisor: &Self, limits: McpSchemaLimits) -> Result<bool> {
        if divisor.zero() {
            return Err(McpSchemaError::InvalidSchema);
        }
        if self.zero() {
            return Ok(true);
        }
        let delta = self.exponent - divisor.exponent;
        let numerator_zeros =
            usize::try_from(delta.max(0)).map_err(|_| McpSchemaError::InstanceLimitExceeded)?;
        let denominator_zeros =
            usize::try_from((-delta).max(0)).map_err(|_| McpSchemaError::InstanceLimitExceeded)?;
        if self.digits.len() + numerator_zeros > limits.max_number_expanded_digits
            || divisor.digits.len() + denominator_zeros > limits.max_number_expanded_digits
        {
            return Err(McpSchemaError::InstanceLimitExceeded);
        }
        let mut numerator = self.digits.clone();
        numerator.resize(numerator.len() + numerator_zeros, b'0');
        let mut denominator = divisor.digits.clone();
        denominator.resize(denominator.len() + denominator_zeros, b'0');
        let numerator =
            num_bigint::BigUint::parse_bytes(&numerator, 10).ok_or(McpSchemaError::InvalidJson)?;
        let denominator = num_bigint::BigUint::parse_bytes(&denominator, 10)
            .ok_or(McpSchemaError::InvalidSchema)?;
        Ok((numerator % denominator) == num_bigint::BigUint::default())
    }
}
