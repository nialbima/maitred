//! OTLP's JSON shape for metrics (ARCHITECTURE, History and OpenTelemetry): each sample is one
//! `ResourceMetrics` per line, so `maitred export` mostly concatenates. About ten hand-written types,
//! so 64-bit integers are written as strings, as OTLP/JSON requires; every figure is a gauge, a
//! reading at a moment. Pure: building and writing a line touches nothing.

use serde::{Deserialize, Serialize};

/// One line of the history: what one sample says, about one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetrics {
    pub resource: Resource,
    pub scope_metrics: Vec<ScopeMetrics>,
}

/// The machine: `service.name`, `host.name`, `host.arch`, `os.type` and `maitred.boot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScopeMetrics {
    pub scope: Scope,
    pub metrics: Vec<Metric>,
}

/// The instrumentation scope: maitred itself, and its version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub unit: String,
    pub gauge: Gauge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gauge {
    pub data_points: Vec<NumberDataPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawPoint")]
pub struct NumberDataPoint {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<KeyValue>,
    /// Nanoseconds since the Unix epoch.
    #[serde(with = "as_string")]
    pub time_unix_nano: u64,
    #[serde(flatten)]
    pub value: Number,
}

/// A point's value: `asDouble`, or `asInt` written as a string.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Number {
    #[serde(rename = "asDouble")]
    Double(Finite),
    #[serde(rename = "asInt", with = "as_string")]
    Int(i64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnyValue {
    #[serde(rename = "stringValue")]
    String(String),
    #[serde(rename = "boolValue")]
    Bool(bool),
    #[serde(rename = "intValue", with = "as_string")]
    Int(i64),
    #[serde(rename = "doubleValue")]
    Double(Finite),
}

/// A double that is a number: JSON has no NaN or infinity, and a figure the sampler got by dividing
/// by zero is one it couldn't take, which it leaves out rather than writing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(into = "f64", try_from = "f64")]
pub struct Finite(f64);

impl Finite {
    /// The value, unless it is NaN or infinite.
    #[must_use]
    pub fn new(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self(value))
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<Finite> for f64 {
    fn from(finite: Finite) -> Self {
        finite.0
    }
}

impl TryFrom<f64> for Finite {
    type Error = &'static str;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("a double must be finite")
    }
}

/// A point as read: OTLP's value is a oneof, so a point with both values or neither is refused.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPoint {
    #[serde(default)]
    attributes: Vec<KeyValue>,
    #[serde(with = "as_string")]
    time_unix_nano: u64,
    as_double: Option<Finite>,
    as_int: Option<Int>,
}

#[derive(Deserialize)]
struct Int(#[serde(with = "as_string")] i64);

impl TryFrom<RawPoint> for NumberDataPoint {
    type Error = &'static str;

    fn try_from(raw: RawPoint) -> Result<Self, Self::Error> {
        let value = match (raw.as_double, raw.as_int) {
            (Some(double), None) => Number::Double(double),
            (None, Some(Int(int))) => Number::Int(int),
            _ => return Err("a point has exactly one of asDouble and asInt"),
        };
        Ok(Self {
            attributes: raw.attributes,
            time_unix_nano: raw.time_unix_nano,
            value,
        })
    }
}

impl ResourceMetrics {
    /// One sample's metrics, from maitred at `version`, about the machine `resource` describes.
    #[must_use]
    pub fn new(resource: Vec<KeyValue>, version: &str, metrics: Vec<Metric>) -> Self {
        Self {
            resource: Resource {
                attributes: resource,
            },
            scope_metrics: vec![ScopeMetrics {
                scope: Scope {
                    name: "maitred".to_owned(),
                    version: version.to_owned(),
                },
                metrics,
            }],
        }
    }

    /// The history's line for it, without the newline.
    #[must_use]
    pub fn to_line(&self) -> String {
        // Every field is a string, a finite number or a bool, and every map key a string, so this
        // can't fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Reads a line back.
    ///
    /// # Errors
    ///
    /// The line isn't one this module writes.
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }
}

impl Metric {
    #[must_use]
    pub fn gauge(name: &str, unit: &str, points: Vec<NumberDataPoint>) -> Self {
        Self {
            name: name.to_owned(),
            unit: unit.to_owned(),
            gauge: Gauge {
                data_points: points,
            },
        }
    }
}

impl NumberDataPoint {
    #[must_use]
    pub fn new(time_unix_nano: u64, value: Number) -> Self {
        Self {
            attributes: Vec::new(),
            time_unix_nano,
            value,
        }
    }

    /// The point with one more attribute.
    #[must_use]
    pub fn with(mut self, attribute: KeyValue) -> Self {
        self.attributes.push(attribute);
        self
    }
}

impl KeyValue {
    #[must_use]
    pub fn string(key: &str, value: &str) -> Self {
        Self {
            key: key.to_owned(),
            value: AnyValue::String(value.to_owned()),
        }
    }

    #[must_use]
    pub fn int(key: &str, value: i64) -> Self {
        Self {
            key: key.to_owned(),
            value: AnyValue::Int(value),
        }
    }

    #[must_use]
    pub fn bool(key: &str, value: bool) -> Self {
        Self {
            key: key.to_owned(),
            value: AnyValue::Bool(value),
        }
    }

    /// The attribute alone, as OTLP/JSON writes it.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// A 64-bit integer written as a decimal string, and read from one (with no leading `+`, as
/// proto3's JSON mapping requires) or from a number.
mod as_string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub(super) fn serialize<T: Display, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(value)
    }

    pub(super) fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: FromStr + TryFrom<u64> + TryFrom<i64>,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        // An untagged enum reads through serde's buffered content, which has no i128: one arm per
        // width it does have.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            String(String),
            Unsigned(u64),
            Signed(i64),
        }
        match Either::deserialize(d)? {
            Either::String(s) if s.starts_with('+') => {
                Err(Error::custom("a leading + isn't allowed"))
            }
            Either::String(s) => s.parse().map_err(Error::custom),
            Either::Unsigned(n) => T::try_from(n).map_err(|_| Error::custom("out of range")),
            Either::Signed(n) => T::try_from(n).map_err(|_| Error::custom("out of range")),
        }
    }
}

#[cfg(test)]
mod tests;
