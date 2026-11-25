//! Tool arguments that are read only after the scope check (bridge phase 2, security review M5).
//!
//! rmcp parses a tool's `Parameters<P>` before the tool method runs, so with a plain `P` a malformed
//! call from a token without the tool's scope got a parse error, not the scope error, and serde's
//! message quoted the value the caller sent. Every tool takes `Parameters<Checked<P>>` instead: rmcp
//! fills it with the raw JSON without reading it (it cannot fail), the tool checks the scope first,
//! and only then [`Checked::read`] turns it into `P`, with a fixed message on failure.

use std::marker::PhantomData;

use rmcp::ErrorData;
use serde::{de::DeserializeOwned, Deserialize, Deserializer};

/// A tool's arguments: raw JSON from a client (read after the scope check), or already typed (tests).
/// Its schema is `P`'s, so clients see the real input shape.
pub enum Checked<P> {
    Raw(serde_json::Value, PhantomData<fn() -> P>),
    Ready(P),
}

impl<'de, P> Deserialize<'de> for Checked<P> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Checked::Raw(serde_json::Value::deserialize(d)?, PhantomData))
    }
}

impl<P> From<P> for Checked<P> {
    fn from(p: P) -> Self {
        Checked::Ready(p)
    }
}

impl<P: schemars::JsonSchema> schemars::JsonSchema for Checked<P> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        P::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        P::json_schema(generator)
    }
}

impl<P: DeserializeOwned> Checked<P> {
    /// The arguments as `P`, or invalid params saying what the tool takes. The message is fixed:
    /// serde's own message can quote a value the caller sent, and a value can be text from
    /// anywhere (an email the client was syncing).
    pub(crate) fn read(self, shape: &'static str) -> Result<P, ErrorData> {
        match self {
            Checked::Raw(raw, _) => serde_json::from_value(raw).map_err(|_| ErrorData::invalid_params(shape, None)),
            Checked::Ready(p) => Ok(p),
        }
    }
}
