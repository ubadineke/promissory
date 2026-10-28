//! Subscribe links (docs/api.md §1): `https://<host>/s#v=1&net=main&to=<UA>&amt=<zat>&…`.
//! The parameters live in the URL fragment so the host serving the page never sees them.

use anyhow::{anyhow, bail, Result};

use crate::network::SoNetwork;

pub const MEMO_MAX_BYTES: usize = 400;
pub const MAX_INSTALLMENTS: u32 = 120;
pub const MIN_EVERY_SECS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscribe {
    pub net: Net,
    pub to: String,
    pub amount_zat: u64,
    pub every_secs: u64,
    pub count: u32,
    pub memo: Option<String>,
    pub start: Option<u64>,
    pub keeper: Option<String>,
    pub hook: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    Main,
    Regtest,
}

impl Net {
    pub fn as_str(&self) -> &'static str {
        match self {
            Net::Main => "main",
            Net::Regtest => "regtest",
        }
    }

    pub fn matches(&self, params: &SoNetwork) -> bool {
        matches!(
            (self, params),
            (Net::Main, SoNetwork::Main) | (Net::Regtest, SoNetwork::Regtest(_))
        )
    }
}

fn decode(v: &str) -> Result<String> {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = v
                    .get(i + 1..i + 3)
                    .ok_or_else(|| anyhow!("bad percent-encoding"))?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| anyhow!("bad percent-encoding"))?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| anyhow!("parameter is not UTF-8"))
}

fn encode(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Subscribe {
    /// Parses a full link or just its fragment (with or without the leading `#`).
    pub fn parse(link: &str) -> Result<Self> {
        let fragment = match link.split_once('#') {
            Some((_, f)) => f,
            None => link,
        };
        let mut get = std::collections::HashMap::new();
        for pair in fragment.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            get.insert(decode(k)?, decode(v)?);
        }
        let req = |name: &str| {
            get.get(name)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("missing parameter `{name}`"))
        };
        let int = |name: &str| -> Result<u64> {
            req(name)?
                .parse::<u64>()
                .map_err(|_| anyhow!("parameter `{name}` must be a non-negative integer"))
        };

        if req("v")? != "1" {
            bail!("parameter `v`: unsupported link version");
        }
        let net = match req("net")?.as_str() {
            "main" => Net::Main,
            "regtest" => Net::Regtest,
            _ => bail!("parameter `net` must be `main` or `regtest`"),
        };
        let to = req("to")?.clone();
        let amount_zat = int("amt")?;
        if amount_zat == 0 {
            bail!("parameter `amt` must be greater than zero");
        }
        let every_secs = int("every")?;
        if every_secs < MIN_EVERY_SECS {
            bail!("parameter `every` must be at least {MIN_EVERY_SECS} seconds");
        }
        let count = u32::try_from(int("n")?).unwrap_or(u32::MAX);
        if !(1..=MAX_INSTALLMENTS).contains(&count) {
            bail!("parameter `n` must be between 1 and {MAX_INSTALLMENTS}");
        }
        let memo = get.get("memo").filter(|m| !m.is_empty()).cloned();
        if memo.as_ref().is_some_and(|m| m.len() > MEMO_MAX_BYTES) {
            bail!("parameter `memo` is longer than {MEMO_MAX_BYTES} bytes");
        }
        let start = match get.get("start").filter(|s| !s.is_empty()) {
            Some(_) => Some(int("start")?),
            None => None,
        };
        Ok(Self {
            net,
            to,
            amount_zat,
            every_secs,
            count,
            memo,
            start,
            keeper: get.get("keeper").filter(|s| !s.is_empty()).cloned(),
            hook: get.get("hook").filter(|s| !s.is_empty()).cloned(),
        })
    }

    /// The link for `base` (e.g. `https://example.com/s`).
    pub fn to_link(&self, base: &str) -> String {
        let mut parts = vec![
            "v=1".to_string(),
            format!("net={}", self.net.as_str()),
            format!("to={}", encode(&self.to)),
            format!("amt={}", self.amount_zat),
            format!("every={}", self.every_secs),
            format!("n={}", self.count),
        ];
        if let Some(m) = &self.memo {
            parts.push(format!("memo={}", encode(m)));
        }
        if let Some(s) = self.start {
            parts.push(format!("start={s}"));
        }
        if let Some(k) = &self.keeper {
            parts.push(format!("keeper={}", encode(k)));
        }
        if let Some(h) = &self.hook {
            parts.push(format!("hook={}", encode(h)));
        }
        format!("{base}#{}", parts.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Subscribe {
        Subscribe {
            net: Net::Regtest,
            to: "uregtest13543hcxsg0xcd7he0nsux3kmz2rqdg905n69x86mlyth67pesmxjpkfg6mwesu9chlpggf6clg58n7zth7uhd4ud9n655l8hkykuwwxj".into(),
            amount_zat: 1_000_000,
            every_secs: 604_800,
            count: 12,
            memo: Some("Café & co — 100%".into()),
            start: None,
            keeper: Some("https://keeper.example/".into()),
            hook: None,
        }
    }

    #[test]
    fn round_trips() {
        let s = sample();
        let link = s.to_link("https://so.example/s");
        assert!(link.starts_with("https://so.example/s#v=1&net=regtest&to=uregtest1"));
        assert_eq!(Subscribe::parse(&link).unwrap(), s);
    }

    #[test]
    fn accepts_bare_fragment_and_ignores_unknown() {
        let link = sample().to_link("x");
        let fragment = link.split_once('#').unwrap().1;
        assert_eq!(Subscribe::parse(&format!("#{fragment}&future=1")).unwrap(), sample());
    }

    #[test]
    fn names_the_bad_parameter() {
        let ok = sample().to_link("x");
        for (from, to, needle) in [
            ("&n=12", "&n=0", "`n`"),
            ("&n=12", "&n=121", "`n`"),
            ("&every=604800", "&every=59", "`every`"),
            ("&amt=1000000", "&amt=0", "`amt`"),
            ("&amt=1000000", "&amt=1.5", "`amt`"),
            ("net=regtest", "net=test", "`net`"),
            ("v=1", "v=2", "`v`"),
        ] {
            let err = Subscribe::parse(&ok.replace(from, to)).unwrap_err().to_string();
            assert!(err.contains(needle), "{to}: {err}");
        }
        let err = Subscribe::parse("#v=1&net=main&amt=1&every=60&n=1").unwrap_err();
        assert!(err.to_string().contains("`to`"));
    }

    #[test]
    fn raw_plus_is_a_space() {
        let link = format!("{}&memo=Pro+plan%2B", sample().to_link("x").replace("&memo=", "&old="));
        assert_eq!(Subscribe::parse(&link).unwrap().memo.as_deref(), Some("Pro plan+"));
    }

    #[test]
    fn memo_limit_is_bytes() {
        let mut s = sample();
        s.memo = Some("é".repeat(201)); // 402 bytes
        assert!(Subscribe::parse(&s.to_link("x")).unwrap_err().to_string().contains("`memo`"));
    }
}
