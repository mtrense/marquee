//! Value types of a theme: [`Color`], [`Length`] and [`FontWeight`].

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB color with alpha, written `#rgb`, `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Opacity, `255` being opaque.
    pub a: u8,
}

impl Color {
    /// An opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A color with opacity `a`.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Whether the color is fully opaque.
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }
}

/// A string that is not a hex color.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color `{0}`: expected `#rgb`, `#rrggbb` or `#rrggbbaa`")]
pub struct ColorError(String);

impl FromStr for Color {
    type Err = ColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ColorError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map_err(|_| err());
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err());
        match hex.len() {
            3 => Ok(Self::rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
            6 => Ok(Self::rgb(pair(0)?, pair(2)?, pair(4)?)),
            8 => Ok(Self::rgba(pair(0)?, pair(2)?, pair(4)?, pair(6)?)),
            _ => Err(err()),
        }
    }
}

/// Lowercase `#rrggbb`, or `#rrggbbaa` when not opaque.
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if !self.is_opaque() {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Color {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A hex color: `#rgb`, `#rrggbb` or `#rrggbbaa`.",
            "type": "string",
            "pattern": "^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$"
        })
    }
}

/// The unit of a [`Length`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    /// Relative to the font size of the element (in headings: the body font size).
    Em,
    /// Relative to the body font size.
    Rem,
    /// Typographic points (1/72 inch).
    Pt,
    /// CSS pixels (1/96 inch).
    Px,
}

impl LengthUnit {
    /// The unit as written after the number: `em`, `rem`, `pt` or `px`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Em => "em",
            Self::Rem => "rem",
            Self::Pt => "pt",
            Self::Px => "px",
        }
    }
}

/// A non-negative length such as `2em` or `11pt`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    pub value: f32,
    pub unit: LengthUnit,
}

impl Length {
    pub const fn new(value: f32, unit: LengthUnit) -> Self {
        Self { value, unit }
    }
}

/// A string that is not a length.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid length `{0}`: expected a number with a unit, e.g. `1.5em`, `1rem`, `11pt`, `16px`"
)]
pub struct LengthError(String);

impl FromStr for Length {
    type Err = LengthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || LengthError(s.to_owned());
        let split = s.find(|c: char| c.is_ascii_alphabetic()).ok_or_else(err)?;
        let (number, unit) = s.split_at(split);
        let (int, frac) = number.split_once('.').unwrap_or((number, "0"));
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !digits(int) || !digits(frac) {
            return Err(err());
        }
        let unit = match unit {
            "em" => LengthUnit::Em,
            "rem" => LengthUnit::Rem,
            "pt" => LengthUnit::Pt,
            "px" => LengthUnit::Px,
            _ => return Err(err()),
        };
        let value = number.parse().map_err(|_| err())?;
        Ok(Self { value, unit })
    }
}

/// The number without trailing zeros, then the unit: `2em`, `0.875em`.
impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.as_str())
    }
}

impl Serialize for Length {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Length {
    fn schema_name() -> Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A number with a unit: `em`, `rem` (relative to the body font size), `pt` or `px`.",
            "type": "string",
            "pattern": "^[0-9]+(\\.[0-9]+)?(em|rem|pt|px)$"
        })
    }
}

/// A font weight, from `thin` (100) to `black` (900).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FontWeight {
    /// 100.
    Thin,
    /// 300.
    Light,
    /// 400.
    Normal,
    /// 500.
    Medium,
    /// 600.
    Semibold,
    /// 700.
    Bold,
    /// 800.
    Extrabold,
    /// 900.
    Black,
}

impl FontWeight {
    /// The CSS/OpenType weight number, 100–900.
    pub const fn number(self) -> u16 {
        match self {
            Self::Thin => 100,
            Self::Light => 300,
            Self::Normal => 400,
            Self::Medium => 500,
            Self::Semibold => 600,
            Self::Bold => 700,
            Self::Extrabold => 800,
            Self::Black => 900,
        }
    }

    /// Whether a terminal should show the weight as bold (600 and up).
    pub const fn is_bold(self) -> bool {
        self.number() >= 600
    }
}
