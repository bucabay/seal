//! Policy: allow, deny, or ask a human.
//!
//! Evaluated *after* a request has been checked against its endpoint
//! definition and *before* the secret is touched, so a denied call never
//! reaches the credential.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A value pulled out of a request body, for a condition to test.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
}

impl Value {
    pub fn from_json(v: &serde_json::Value) -> Option<Value> {
        match v {
            serde_json::Value::Number(n) => n.as_f64().map(Value::Num),
            serde_json::Value::String(s) => Some(Value::Str(s.clone())),
            serde_json::Value::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Value {
    /// How a value appears to a person — in an approval prompt, for instance.
    /// `Debug` renders `Num(500000.0)`, which is noise to everyone but a
    /// compiler.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::Str(s) => f.write_str(s),
            Value::Bool(b) => write!(f, "{}", b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Ne,
}

impl Op {
    fn parse(s: &str) -> Option<Op> {
        Some(match s {
            ">" => Op::Gt,
            ">=" => Op::Gte,
            "<" => Op::Lt,
            "<=" => Op::Lte,
            "==" | "=" => Op::Eq,
            "!=" => Op::Ne,
            _ => return None,
        })
    }
}

/// `<field> <op> <literal>`, e.g. `amount > 100000`.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub field: String,
    pub op: Op,
    pub rhs: Value,
}

impl Condition {
    pub fn parse(src: &str) -> Result<Condition, String> {
        let parts: Vec<&str> = src.split_whitespace().collect();
        if parts.len() < 3 {
            return Err(format!("expected `field op value`, got `{}`", src));
        }
        let field = parts[0].to_string();
        let op = Op::parse(parts[1]).ok_or_else(|| format!("unknown operator `{}`", parts[1]))?;
        let raw = parts[2..].join(" ");
        let rhs = if let Ok(n) = raw.parse::<f64>() {
            Value::Num(n)
        } else if raw == "true" || raw == "false" {
            Value::Bool(raw == "true")
        } else {
            Value::Str(raw.trim_matches(|c| c == '"' || c == '\'').to_string())
        };
        Ok(Condition { field, op, rhs })
    }

    /// A condition over a field that is absent is false, never an error: a
    /// missing field must not silently satisfy a `deny` rule.
    pub fn eval(&self, facts: &BTreeMap<String, Value>) -> bool {
        let Some(lhs) = facts.get(&self.field) else {
            return false;
        };
        match (lhs, &self.rhs) {
            (Value::Num(a), Value::Num(b)) => match self.op {
                Op::Gt => a > b,
                Op::Gte => a >= b,
                Op::Lt => a < b,
                Op::Lte => a <= b,
                Op::Eq => a == b,
                Op::Ne => a != b,
            },
            (Value::Str(a), Value::Str(b)) => match self.op {
                Op::Eq => a == b,
                Op::Ne => a != b,
                _ => false, // ordering on strings is not defined here
            },
            (Value::Bool(a), Value::Bool(b)) => match self.op {
                Op::Eq => a == b,
                Op::Ne => a != b,
                _ => false,
            },
            // Type mismatch never matches, and never errors.
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    StepUp(String),
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// Rules attached to an endpoint. Absent rules mean "no opinion", which
/// resolves to allow — the endpoint definition has already constrained the
/// request by that point.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Refuse outright when this holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<String>,
    /// Require a human before proceeding when this holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_up: Option<String>,
    /// Require a human for every call, regardless of contents.
    #[serde(default)]
    pub always_step_up: bool,
}

impl Policy {
    /// Deny wins over step-up, and step-up wins over allow. A malformed rule
    /// fails closed: it denies rather than being skipped.
    pub fn evaluate(&self, facts: &BTreeMap<String, Value>) -> Decision {
        if let Some(src) = &self.deny {
            match Condition::parse(src) {
                Ok(c) if c.eval(facts) => return Decision::Deny(src.clone()),
                Err(e) => return Decision::Deny(format!("unparseable deny rule: {}", e)),
                _ => {}
            }
        }
        if self.always_step_up {
            return Decision::StepUp("always".into());
        }
        if let Some(src) = &self.step_up {
            match Condition::parse(src) {
                Ok(c) if c.eval(facts) => return Decision::StepUp(src.clone()),
                Err(e) => return Decision::Deny(format!("unparseable step_up rule: {}", e)),
                _ => {}
            }
        }
        Decision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn parses_numeric_conditions() {
        let c = Condition::parse("amount > 100000").unwrap();
        assert_eq!(c.field, "amount");
        assert_eq!(c.op, Op::Gt);
        assert_eq!(c.rhs, Value::Num(100000.0));
    }

    #[test]
    fn parses_string_and_bool_conditions() {
        assert_eq!(
            Condition::parse("currency != usd").unwrap().rhs,
            Value::Str("usd".into())
        );
        assert_eq!(
            Condition::parse(r#"mode == "live""#).unwrap().rhs,
            Value::Str("live".into())
        );
        assert_eq!(
            Condition::parse("livemode == true").unwrap().rhs,
            Value::Bool(true)
        );
    }

    #[test]
    fn rejects_malformed_conditions() {
        assert!(Condition::parse("amount").is_err());
        assert!(Condition::parse("amount >").is_err());
        assert!(Condition::parse("amount ~~ 5").is_err());
    }

    #[test]
    fn numeric_comparisons_work() {
        let f = facts(&[("amount", Value::Num(150_000.0))]);
        assert!(Condition::parse("amount > 100000").unwrap().eval(&f));
        assert!(!Condition::parse("amount < 100000").unwrap().eval(&f));
        assert!(Condition::parse("amount >= 150000").unwrap().eval(&f));
        assert!(Condition::parse("amount != 1").unwrap().eval(&f));
    }

    #[test]
    fn a_missing_field_is_false_not_an_error() {
        let f = facts(&[("other", Value::Num(1.0))]);
        assert!(!Condition::parse("amount > 1").unwrap().eval(&f));
        assert!(
            !Condition::parse("amount != 1").unwrap().eval(&f),
            "absent must not satisfy a not-equals deny rule"
        );
    }

    #[test]
    fn type_mismatch_never_matches() {
        let f = facts(&[("amount", Value::Str("lots".into()))]);
        assert!(!Condition::parse("amount > 100").unwrap().eval(&f));
    }

    #[test]
    fn no_rules_means_allow() {
        assert_eq!(Policy::default().evaluate(&facts(&[])), Decision::Allow);
    }

    #[test]
    fn step_up_triggers_on_its_condition() {
        let p = Policy {
            step_up: Some("amount > 100000".into()),
            ..Default::default()
        };
        assert_eq!(
            p.evaluate(&facts(&[("amount", Value::Num(5.0))])),
            Decision::Allow
        );
        assert!(matches!(
            p.evaluate(&facts(&[("amount", Value::Num(200_000.0))])),
            Decision::StepUp(_)
        ));
    }

    #[test]
    fn deny_beats_step_up() {
        let p = Policy {
            deny: Some("mode == live".into()),
            step_up: Some("amount > 1".into()),
            ..Default::default()
        };
        let f = facts(&[
            ("mode", Value::Str("live".into())),
            ("amount", Value::Num(10.0)),
        ]);
        assert!(matches!(p.evaluate(&f), Decision::Deny(_)));
    }

    #[test]
    fn always_step_up_applies_with_no_condition() {
        let p = Policy {
            always_step_up: true,
            ..Default::default()
        };
        assert_eq!(p.evaluate(&facts(&[])), Decision::StepUp("always".into()));
    }

    #[test]
    fn a_broken_rule_fails_closed() {
        let p = Policy {
            deny: Some("garbage".into()),
            ..Default::default()
        };
        assert!(
            matches!(p.evaluate(&facts(&[])), Decision::Deny(_)),
            "an unparseable rule must deny, never be skipped"
        );
        let p2 = Policy {
            step_up: Some("also garbage".into()),
            ..Default::default()
        };
        assert!(matches!(p2.evaluate(&facts(&[])), Decision::Deny(_)));
    }

    #[test]
    fn values_render_for_people_not_for_compilers() {
        assert_eq!(Value::Num(500_000.0).to_string(), "500000");
        assert_eq!(Value::Num(1.5).to_string(), "1.5");
        assert_eq!(Value::Str("ch_1".into()).to_string(), "ch_1");
        assert_eq!(Value::Bool(true).to_string(), "true");
    }

    #[test]
    fn values_come_out_of_json() {
        assert_eq!(
            Value::from_json(&serde_json::json!(5)),
            Some(Value::Num(5.0))
        );
        assert_eq!(
            Value::from_json(&serde_json::json!("x")),
            Some(Value::Str("x".into()))
        );
        assert_eq!(
            Value::from_json(&serde_json::json!(true)),
            Some(Value::Bool(true))
        );
        assert_eq!(Value::from_json(&serde_json::json!(null)), None);
    }
}
