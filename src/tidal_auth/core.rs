//! Deterministic token validation and renewal decisions.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const MAX_TOKEN_BYTES: usize = 16 * 1024;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const RENEW_BEFORE_SECONDS: i64 = 300;
pub const MAX_LIFETIME_SECONDS: i64 = 31 * 24 * 60 * 60;

#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub user_id: i64,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub user_id: i64,
}

#[derive(Serialize)]
pub struct ExportAccess<'a> {
    access_token: &'a str,
    expires_at: Option<DateTime<Utc>>,
}

impl Access {
    pub fn export(&self) -> ExportAccess<'_> {
        ExportAccess {
            access_token: &self.access_token,
            expires_at: self.expires_at,
        }
    }

    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), &'static str> {
        if !valid_token(&self.access_token)
            || self.user_id <= 0
            || self.expires_at.is_none_or(|expiry| expiry <= now)
        {
            return Err("invalid_broker_reply");
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Get,
    Refresh { rejected_access_token: String },
}

pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._~+/=-".contains(&c))
}

impl Credentials {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_token(&self.access_token)
            || !valid_token(&self.refresh_token)
            || !self.token_type.eq_ignore_ascii_case("Bearer")
            || self.user_id <= 0
            || self.client_id.as_ref().is_some_and(|id| !valid_token(id))
        {
            return Err("invalid_credentials");
        }
        Ok(())
    }
    pub fn access(&self) -> Access {
        Access {
            access_token: self.access_token.clone(),
            expires_at: self.expires_at,
            user_id: self.user_id,
        }
    }
    pub fn needs_refresh(
        &self,
        request: &Request,
        now: DateTime<Utc>,
    ) -> Result<bool, &'static str> {
        self.validate()?;
        if let Request::Refresh {
            rejected_access_token,
        } = request
        {
            if !valid_token(rejected_access_token) {
                return Err("invalid_request");
            }
            if rejected_access_token == &self.access_token {
                return Ok(true);
            }
        }
        let renewal_boundary = now
            .checked_add_signed(Duration::seconds(RENEW_BEFORE_SECONDS))
            .ok_or("invalid_clock")?;
        Ok(self
            .expires_at
            .is_none_or(|expiry| expiry <= renewal_boundary))
    }
}

#[derive(Deserialize)]
struct Renewal {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
    token_type: String,
}

pub fn renewed(
    current: &Credentials,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<Credentials, &'static str> {
    if body.len() > MAX_FRAME_BYTES {
        return Err("oversized_reply");
    }
    let renewal: Renewal = serde_json::from_slice(body).map_err(|_| "invalid_refresh_reply")?;
    if renewal.expires_in <= RENEW_BEFORE_SECONDS || renewal.expires_in > MAX_LIFETIME_SECONDS {
        return Err("invalid_token_lifetime");
    }
    let mut next = current.clone();
    next.access_token = renewal.access_token;
    if let Some(refresh) = renewal.refresh_token {
        next.refresh_token = refresh;
    }
    next.token_type = renewal.token_type;
    next.expires_at = now.checked_add_signed(Duration::seconds(renewal.expires_in));
    if next.expires_at.is_none() {
        return Err("invalid_token_lifetime");
    }
    next.validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Credentials {
        Credentials {
            access_token: "old-access".into(),
            refresh_token: "private-refresh".into(),
            token_type: "Bearer".into(),
            user_id: 1,
            expires_at: None,
            client_id: None,
        }
    }
    #[test]
    fn expiry_and_rejected_token_decisions() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        let mut value = fixture();
        assert!(value.needs_refresh(&Request::Get, now).unwrap());
        assert_eq!(
            value.needs_refresh(&Request::Get, DateTime::<Utc>::MAX_UTC),
            Err("invalid_clock")
        );
        value.expires_at = Some(now + Duration::seconds(MAX_LIFETIME_SECONDS));
        assert!(!value.needs_refresh(&Request::Get, now).unwrap());
        assert!(value
            .needs_refresh(
                &Request::Refresh {
                    rejected_access_token: value.access_token.clone()
                },
                now
            )
            .unwrap());
        value.access_token = "new-access".into();
        assert!(!value
            .needs_refresh(
                &Request::Refresh {
                    rejected_access_token: "old-access".into()
                },
                now
            )
            .unwrap());
        value.expires_at = Some(now);
        assert!(value.needs_refresh(&Request::Get, now).unwrap());
        assert!(value
            .needs_refresh(
                &Request::Refresh {
                    rejected_access_token: "bad\nheader".into()
                },
                now
            )
            .is_err());
    }
    #[test]
    fn complete_renewal_only_and_no_refresh_in_access() {
        let current = fixture();
        let now = DateTime::<Utc>::UNIX_EPOCH;
        let good = br#"{"access_token":"new-access","token_type":"Bearer","expires_in":3600}"#;
        let next = renewed(&current, good, now).unwrap();
        assert_eq!(next.refresh_token, current.refresh_token);
        assert!(next.access().validate(now).is_ok());
        let access = next.access();
        let exported = serde_json::to_value(access.export()).unwrap();
        assert_eq!(exported["access_token"], next.access_token);
        assert!(exported.get("expires_at").is_some());
        assert!(exported.get("user_id").is_none());
        assert!(exported.get("refresh_token").is_none());
        assert!(current.access().validate(now).is_err());
        assert!(next.access().validate(next.expires_at.unwrap()).is_err());
        let lowercase = br#"{"access_token":"new-access","token_type":"bearer","expires_in":3600}"#;
        assert!(renewed(&current, lowercase, now).is_ok());
        assert!(renewed(&current, good, DateTime::<Utc>::MAX_UTC).is_err());
        assert!(!serde_json::to_string(&next.access())
            .unwrap()
            .contains("refresh"));
        for bad in [
            b"{}".as_slice(),
            b"[]",
            br#"{"access_token":"new","token_type":"Bearer","expires_in":0}"#,
            br#"{"access_token":"new","refresh_token":"","token_type":"Bearer","expires_in":3600}"#,
        ] {
            assert!(renewed(&current, bad, now).is_err());
        }
        assert!(renewed(&current, &vec![0; MAX_FRAME_BYTES + 1], now).is_err());
        assert_eq!(current.access_token, "old-access");
    }
}
