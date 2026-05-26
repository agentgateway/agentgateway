use std::any::Any;
use std::fmt::Debug;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::common::types;
use crate::common::types::Type;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CelVal {
	Unspecified,
	Error,
	Dyn,
	Any,
	Boolean(bool),
	Bytes(Vec<u8>),
	Double(f64),
	#[serde(serialize_with = "serialize_duration")]
	Duration(Duration),
	Int(i64),
	List,
	Map,
	Null,
	String(String),
	#[serde(serialize_with = "serialize_system_time")]
	Timestamp(SystemTime),
	Type,
	UInt(u64),
	Unknown,
}

fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	#[derive(serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	struct DurationSerde {
		secs: u64,
		nanos: u32,
	}

	DurationSerde {
		secs: duration.as_secs(),
		nanos: duration.subsec_nanos(),
	}
	.serialize(serializer)
}

fn serialize_system_time<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let time: chrono::DateTime<chrono::Utc> = (*time).into();
	serializer.serialize_str(&time.to_rfc3339())
}

pub trait Val {
	fn get_type(&self) -> Type<'_>;

	fn into_inner(self) -> Box<dyn Any>;
}

impl Val for CelVal {
	fn get_type(&self) -> Type<'_> {
		match self {
			CelVal::Unspecified => Type::new_unspecified_type("unspecified"),
			CelVal::Error => types::ERROR_TYPE,
			CelVal::Dyn => types::DYN_TYPE,
			CelVal::Any => types::ANY_TYPE,
			CelVal::Boolean(_) => types::BOOL_TYPE,
			CelVal::Bytes(_) => types::BYTES_TYPE,
			CelVal::Double(_) => types::DOUBLE_TYPE,
			CelVal::Duration(_) => types::DURATION_TYPE,
			CelVal::Int(_) => types::INT_TYPE,
			CelVal::List => types::LIST_TYPE,
			CelVal::Map => types::MAP_TYPE,
			CelVal::Null => types::NULL_TYPE,
			CelVal::String(_) => types::STRING_TYPE,
			CelVal::Timestamp(_) => types::TIMESTAMP_TYPE,
			CelVal::Type => types::TYPE_TYPE,
			CelVal::UInt(_) => types::UINT_TYPE,
			CelVal::Unknown => types::UNKNOWN_TYPE,
		}
	}

	fn into_inner(self) -> Box<dyn Any> {
		match self {
			CelVal::Unspecified => todo!(),
			CelVal::Error => todo!(),
			CelVal::Dyn => todo!(),
			CelVal::Any => todo!(),
			CelVal::Boolean(b) => Box::new(b),
			CelVal::Bytes(b) => Box::new(b),
			CelVal::Double(d) => Box::new(d),
			CelVal::Duration(d) => Box::new(d),
			CelVal::Int(i) => Box::new(i),
			CelVal::List => todo!(),
			CelVal::Map => todo!(),
			CelVal::Null => todo!(),
			CelVal::String(s) => Box::new(s),
			CelVal::Timestamp(t) => Box::new(t),
			CelVal::Type => todo!(),
			CelVal::UInt(u) => Box::new(u),
			CelVal::Unknown => todo!(),
		}
	}
}
