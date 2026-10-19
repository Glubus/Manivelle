//! `chrono` date and time types cross as the strings their `serde` impls write:
//! RFC 3339 for `DateTime` (`Z` for a zero offset), ISO 8601 for the naive types.
//! Decoding accepts what those impls parse from a JSON string.

use std::fmt::{self, Display, Write};

use ::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc};
use rquickjs::{Ctx, Value as JsValue};

use super::super::text::{decode_parsed, encode_display};
use super::super::{CodecError, JsDecode, JsEncode};
use crate::contract::{TsSchema, TsType};

/// What a `DateTime` string must be, for decode errors.
const DATE_TIME_EXPECTED: &str = "an RFC 3339 date and time";

macro_rules! naive_codecs {
    ($($ty:ident => $format:literal, $expected:literal),+ $(,)?) => {
        $(
            impl JsEncode for $ty {
                fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
                    encode_display(ctx, &format_args!($format, self), stringify!($ty))
                }
            }

            impl JsDecode for $ty {
                fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
                    decode_parsed(value, stringify!($ty), $expected)
                }
            }

            impl TsSchema for $ty {
                fn ts_type() -> TsType {
                    TsType::String
                }
            }
        )+
    };
}

// `serde` writes the dates through `Debug` and the time through `Display`.
naive_codecs!(
    NaiveDate => "{:?}", "an ISO 8601 date",
    NaiveTime => "{}", "an ISO 8601 time",
    NaiveDateTime => "{:?}", "an ISO 8601 date and time",
);

impl<Tz: TimeZone> JsEncode for DateTime<Tz> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        let offset = Rfc3339Offset(self.offset().fix());
        encode_display(
            ctx,
            &format_args!("{:?}{offset}", self.naive_local()),
            "DateTime",
        )
    }
}

impl JsDecode for DateTime<FixedOffset> {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_parsed(value, "DateTime<FixedOffset>", DATE_TIME_EXPECTED)
    }
}

/// Any offset is accepted and converted to UTC, as `serde` does.
impl JsDecode for DateTime<Utc> {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_parsed::<DateTime<FixedOffset>>(value, "DateTime<Utc>", DATE_TIME_EXPECTED)
            .map(|date_time| date_time.with_timezone(&Utc))
    }
}

impl<Tz: TimeZone> TsSchema for DateTime<Tz> {
    fn ts_type() -> TsType {
        TsType::String
    }
}

/// UTC offset the way `DateTime`'s `serde` impl writes it: `Z` when zero, otherwise
/// `±hh:mm` with seconds rounded to the nearest minute.
struct Rfc3339Offset(FixedOffset);

impl Display for Rfc3339Offset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.0.local_minus_utc();
        if seconds == 0 {
            return formatter.write_char('Z');
        }
        let sign = if seconds < 0 { '-' } else { '+' };
        let minutes = (seconds.unsigned_abs() + 30) / 60;
        write!(formatter, "{sign}{:02}:{:02}", minutes / 60, minutes % 60)
    }
}
