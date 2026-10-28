//! The tools, their input schemas and strict input validation. Pure: no I/O.

use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Map, Value};

/// Stated in every tool description: the agent can never move funds by itself.
pub const CONFIRMATION_RULE: &str = "Confirmation rule: this server never moves funds and holds no keys. \
Any payment, or any change to an allowance, is only a pending request until the payer approves it in \
the Promissory app; nothing is created or paid without that approval. No tool can cancel orders.";

pub const MAX_MONEY_ZAT: u64 = 21_000_000 * 100_000_000;
pub const MEMO_MAX_BYTES: usize = 400;
pub const REASON_MAX_CHARS: usize = 200;
/// A one-off may be requested at most this far ahead (as promissory-app enforces).
pub const MAX_DUE_AHEAD_SECS: u64 = 400 * 86_400;
const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

pub const ALLOWANCE_STATUS: &str = "allowance_status";
pub const LIST_ORDERS: &str = "list_orders";
pub const CREATE_ONEOFF: &str = "create_oneoff";
pub const REQUEST_STATUS: &str = "request_status";
pub const REQUEST_ALLOWANCE_CHANGE: &str = "request_allowance_change";

pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub schema: Value,
    pub read_only: bool,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn id_schema(what: &str) -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": i64::MAX, "description": what })
}

fn zat_schema(what: &str) -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": MAX_MONEY_ZAT, "description": what })
}

fn reason_schema() -> Value {
    json!({ "type": "string", "maxLength": REASON_MAX_CHARS, "description": "Why you are asking; shown to the payer." })
}

/// Every tool this server offers.
pub fn all() -> Vec<ToolDef> {
    let rule = CONFIRMATION_RULE;
    vec![
        ToolDef {
            name: ALLOWANCE_STATUS,
            title: "Allowance status",
            description: format!(
                "Read-only. Shows your allowance from the payer: cap per period, period length, how much was \
                 released to you this period (spent) and what remains, when the next chip is released, and \
                 the address it pays (your receiving address). Without `id`, lists every allowance. \
                 Amounts are in zatoshis (1 ZEC = 100000000 zat). {rule}"
            ),
            schema: object(json!({ "id": id_schema("Allowance id. Omit to list all allowances.") }), &[]),
            read_only: true,
        },
        ToolDef {
            name: LIST_ORDERS,
            title: "List standing orders",
            description: format!(
                "Read-only. Summarizes the payer's standing orders: id, status, payee, amount per installment, \
                 cadence, installments paid. {rule}"
            ),
            schema: object(json!({}), &[]),
            read_only: true,
        },
        ToolDef {
            name: CREATE_ONEOFF,
            title: "Request a one-off payment",
            description: format!(
                "Asks the payer to approve one shielded payment. It does NOT pay: it creates a pending request \
                 and returns {{request_id, status: \"pending\", approve_at}}. The payer approves or rejects it \
                 at approve_at; poll request_status. At most 10 requests can wait at once. {rule}"
            ),
            schema: object(
                json!({
                    "payee": { "type": "string", "minLength": 3, "maxLength": 1000,
                               "description": "Unified address (u1…, utest1… or uregtest1…) with an Orchard/Ironwood receiver." },
                    "amount_zat": zat_schema("Amount in zatoshis (1 ZEC = 100000000)."),
                    "due": { "type": "integer", "minimum": 0,
                             "description": "Unix time it should pay, at least a minute ahead and within 400 days. Default: 10 minutes after approval." },
                    "memo": { "type": "string", "maxLength": MEMO_MAX_BYTES, "description": "Memo the payee sees (at most 400 bytes)." },
                    "reason": reason_schema(),
                }),
                &["payee", "amount_zat"],
            ),
            read_only: false,
        },
        ToolDef {
            name: REQUEST_STATUS,
            title: "Request status",
            description: format!(
                "Read-only. The state of a request you made: pending, approved (with the order or allowance it \
                 created), rejected or expired. {rule}"
            ),
            schema: object(json!({ "request_id": id_schema("From create_oneoff or request_allowance_change.") }), &["request_id"]),
            read_only: true,
        },
        ToolDef {
            name: REQUEST_ALLOWANCE_CHANGE,
            title: "Request an allowance change",
            description: format!(
                "Asks the payer to change an allowance's cap per period. It does NOT change anything: it \
                 creates a pending request; if approved, the new cap applies from the next period. \
                 new_cap_zat must be a multiple of the allowance's chips. {rule}"
            ),
            schema: object(
                json!({
                    "id": id_schema("Allowance id."),
                    "new_cap_zat": zat_schema("New cap per period, in zatoshis."),
                    "reason": reason_schema(),
                }),
                &["id", "new_cap_zat"],
            ),
            read_only: false,
        },
    ]
}

// ---------------------------------------------------------------------------------------------
// Arguments

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowanceStatusArgs {
    pub id: Option<u64>,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OneoffArgs {
    pub payee: String,
    pub amount_zat: u64,
    pub due: Option<u64>,
    pub memo: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestStatusArgs {
    pub request_id: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AllowanceChangeArgs {
    pub id: u64,
    pub new_cap_zat: u64,
    pub reason: Option<String>,
}

/// Parses tool arguments strictly: unknown fields, wrong types, negative or fractional numbers
/// and numeric strings are all refused.
pub fn parse<T: DeserializeOwned>(args: Option<Map<String, Value>>) -> Result<T, String> {
    serde_json::from_value(Value::Object(args.unwrap_or_default())).map_err(|e| format!("invalid arguments: {e}"))
}

pub fn check_id(name: &str, id: u64) -> Result<(), String> {
    if id == 0 || id > i64::MAX as u64 {
        return Err(format!("{name} must be a positive integer"));
    }
    Ok(())
}

pub fn check_zat(name: &str, zat: u64) -> Result<(), String> {
    if zat == 0 || zat > MAX_MONEY_ZAT {
        return Err(format!("{name} must be between 1 and {MAX_MONEY_ZAT} zatoshis"));
    }
    Ok(())
}

/// A unified address by shape: known prefix and bech32 characters. promissory-app checks the rest
/// (checksum, network, Orchard receiver).
pub fn check_ua(ua: &str) -> Result<(), String> {
    let err = || Err("payee must be a unified address (u1…, utest1… or uregtest1…)".to_string());
    let Some(data) = ["uregtest1", "utest1", "u1"].iter().find_map(|p| ua.strip_prefix(p)) else {
        return err();
    };
    if ua.len() > 1000 || data.len() < 20 || !data.chars().all(|c| BECH32_CHARSET.contains(c)) {
        return err();
    }
    Ok(())
}

fn check_text(name: &str, s: &Option<String>, max_chars: usize, max_bytes: usize) -> Result<(), String> {
    if let Some(s) = s {
        if s.chars().count() > max_chars || s.len() > max_bytes {
            return Err(format!("{name} is too long (at most {max_chars} characters)"));
        }
        if s.chars().any(char::is_control) {
            return Err(format!("{name} must not contain control characters"));
        }
    }
    Ok(())
}

impl OneoffArgs {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        check_ua(&self.payee)?;
        check_zat("amount_zat", self.amount_zat)?;
        if let Some(due) = self.due {
            if due < now + 60 {
                return Err("due must be a unix time at least a minute from now".into());
            }
            if due > now + MAX_DUE_AHEAD_SECS {
                return Err("due must be within 400 days".into());
            }
        }
        check_text("memo", &self.memo, MEMO_MAX_BYTES, MEMO_MAX_BYTES)?;
        check_text("reason", &self.reason, REASON_MAX_CHARS, 4 * REASON_MAX_CHARS)
    }
}

impl AllowanceChangeArgs {
    pub fn validate(&self) -> Result<(), String> {
        check_id("id", self.id)?;
        check_zat("new_cap_zat", self.new_cap_zat)?;
        check_text("reason", &self.reason, REASON_MAX_CHARS, 4 * REASON_MAX_CHARS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj";

    fn args(v: Value) -> Option<Map<String, Value>> {
        match v {
            Value::Object(m) => Some(m),
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn every_tool_states_the_confirmation_rule() {
        let tools = all();
        assert_eq!(tools.len(), 5);
        for t in &tools {
            assert!(t.description.contains(CONFIRMATION_RULE), "{}", t.name);
            assert_eq!(t.schema["additionalProperties"], false, "{}", t.name);
        }
        let writes: Vec<_> = tools.iter().filter(|t| !t.read_only).map(|t| t.name).collect();
        assert_eq!(writes, [CREATE_ONEOFF, REQUEST_ALLOWANCE_CHANGE]);
        for t in tools.iter().filter(|t| !t.read_only) {
            assert!(t.description.contains("pending request"), "{}", t.name);
        }
    }

    #[test]
    fn no_args_and_empty_args() {
        assert_eq!(parse::<NoArgs>(None), Ok(NoArgs {}));
        assert_eq!(parse::<AllowanceStatusArgs>(None), Ok(AllowanceStatusArgs { id: None }));
        assert_eq!(parse::<AllowanceStatusArgs>(args(json!({"id": 3}))), Ok(AllowanceStatusArgs { id: Some(3) }));
        assert!(parse::<NoArgs>(args(json!({"x": 1}))).unwrap_err().contains("unknown field"));
    }

    #[test]
    fn oneoff_parsing_is_strict() {
        let ok = parse::<OneoffArgs>(args(json!({"payee": UA, "amount_zat": 1_000_000}))).unwrap();
        assert_eq!(ok.validate(1_000), Ok(()));
        for bad in [
            json!({"payee": UA}),
            json!({"payee": UA, "amount_zat": -1}),
            json!({"payee": UA, "amount_zat": 1.5}),
            json!({"payee": UA, "amount_zat": "1000"}),
            json!({"payee": UA, "amount_zat": 1, "to": "elsewhere"}),
            json!({"payee": 5, "amount_zat": 1}),
            json!({"payee": UA, "amount_zat": 1, "due": -5}),
        ] {
            assert!(parse::<OneoffArgs>(args(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn oneoff_validation() {
        let now = 1_000_000;
        let base = || OneoffArgs {
            payee: UA.into(),
            amount_zat: 1_000_000,
            due: None,
            memo: None,
            reason: None,
        };
        let check = |f: &dyn Fn(&mut OneoffArgs)| {
            let mut a = base();
            f(&mut a);
            a.validate(now)
        };
        assert_eq!(check(&|_| {}), Ok(()));
        assert_eq!(check(&|a| a.due = Some(now + 3_600)), Ok(()));
        assert!(check(&|a| a.amount_zat = 0).is_err());
        assert!(check(&|a| a.amount_zat = MAX_MONEY_ZAT + 1).is_err());
        assert!(check(&|a| a.due = Some(now + 30)).unwrap_err().contains("minute"));
        assert!(check(&|a| a.due = Some(now + MAX_DUE_AHEAD_SECS + 1)).unwrap_err().contains("400 days"));
        assert!(check(&|a| a.memo = Some("x".repeat(401))).unwrap_err().contains("memo"));
        assert!(check(&|a| a.memo = Some("é".repeat(201))).unwrap_err().contains("memo"), "400 bytes, not chars");
        assert!(check(&|a| a.reason = Some("a\u{7}b".into())).unwrap_err().contains("control"));
        assert!(check(&|a| a.payee = "t1abc".into()).is_err());
        assert!(check(&|a| a.payee = "zs1qqqqqqqqqqqqqqqqqqqqqqqqqqqq".into()).is_err());
        assert!(check(&|a| a.payee = "u1BADCHARSqqqqqqqqqqqqqqqqqqqqqqqq".into()).is_err());
        assert!(check(&|a| a.payee = "u1qqq".into()).is_err());
    }

    #[test]
    fn allowance_change_validation() {
        let a = parse::<AllowanceChangeArgs>(args(json!({"id": 2, "new_cap_zat": 3_000_000}))).unwrap();
        assert_eq!(a.validate(), Ok(()));
        assert!(parse::<AllowanceChangeArgs>(args(json!({"id": 2}))).is_err());
        assert!(parse::<AllowanceChangeArgs>(args(json!({"id": 2, "new_cap_zat": 1, "cancel": true}))).is_err());
        let bad = |id, cap| AllowanceChangeArgs { id, new_cap_zat: cap, reason: None }.validate();
        assert!(bad(0, 1).is_err());
        assert!(bad(u64::MAX, 1).is_err());
        assert!(bad(1, 0).is_err());
        assert!(check_id("request_id", 7).is_ok());
    }
}
