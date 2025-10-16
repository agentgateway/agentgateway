use std::fmt::{Debug, Display, Error, Write};
use std::mem;
use std::sync::Arc;

use prometheus_client::encoding::{
	EncodeLabelSet, EncodeLabelValue, LabelSetEncoder, LabelValueEncoder,
};
use prometheus_client::registry::Registry;
use tracing::error;
use tracing::field::{DisplayValue, display};
use tracing_core::field::Value;

use crate::strng::{RichStrng, Strng};

pub const PREFIX: &str = "agentgateway";

/// Creates a metrics sub registry for agentgateway.
pub fn sub_registry(registry: &mut Registry) -> &mut Registry {
	registry.sub_registry_with_prefix(PREFIX)
}

pub struct Deferred<'a, F, T>
where
	F: FnOnce(&'a T),
	T: ?Sized,
{
	param: &'a T,
	deferred_fn: Option<F>,
}

impl<'a, F, T> Deferred<'a, F, T>
where
	F: FnOnce(&'a T),
	T: ?Sized,
{
	pub fn new(param: &'a T, deferred_fn: F) -> Self {
		Self {
			param,
			deferred_fn: Some(deferred_fn),
		}
	}
}

impl<'a, F, T> Drop for Deferred<'a, F, T>
where
	F: FnOnce(&'a T),
	T: ?Sized,
{
	fn drop(&mut self) {
		if let Some(deferred_fn) = mem::take(&mut self.deferred_fn) {
			(deferred_fn)(self.param);
		} else {
			error!("defer deferred record failed, event is gone");
		}
	}
}

pub trait DeferRecorder {
	#[must_use = "metric will be dropped (and thus recorded) immediately if not assigned"]
	/// Perform a record operation on this object when the returned [Deferred] object is
	/// dropped.
	fn defer_record<'a, F>(&'a self, record: F) -> Deferred<'a, F, Self>
	where
		F: FnOnce(&'a Self),
	{
		Deferred::new(self, record)
	}
}

pub trait Recorder<E, T> {
	/// Record the given event
	fn record(&self, event: E, meta: T);
}

pub trait IncrementRecorder<E>: Recorder<E, u64> {
	/// Record the given event by incrementing the counter by count
	fn increment(&self, event: E);
}

impl<E, R> IncrementRecorder<E> for R
where
	R: Recorder<E, u64>,
{
	fn increment(&self, event: E) {
		self.record(event, 1);
	}
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
// DefaultedUnknown is a wrapper around an Option that encodes as "unknown" when missing, rather than ""
pub struct DefaultedUnknown<T>(Option<T>);

impl DefaultedUnknown<RichStrng> {
	pub fn display(&self) -> Option<DisplayValue<&str>> {
		self.as_ref().map(|rs| display(rs.as_str()))
	}
	pub fn to_value(&self) -> Option<impl Value + '_> {
		self.as_ref().map(|rs| rs.as_str())
	}
}

impl<T> DefaultedUnknown<T> {
	pub fn inner(self) -> Option<T> {
		self.0
	}
	pub fn as_ref(&self) -> Option<&T> {
		self.0.as_ref()
	}
}

impl<T> Default for DefaultedUnknown<T> {
	fn default() -> Self {
		Self(None)
	}
}

// Surely there is a less verbose way to do this, but I cannot find one.

impl From<String> for DefaultedUnknown<String> {
	fn from(t: String) -> Self {
		if t.is_empty() {
			DefaultedUnknown(None)
		} else {
			DefaultedUnknown(Some(t))
		}
	}
}

impl From<RichStrng> for DefaultedUnknown<RichStrng> {
	fn from(t: RichStrng) -> Self {
		if t.is_empty() {
			DefaultedUnknown(None)
		} else {
			DefaultedUnknown(Some(t))
		}
	}
}

impl From<String> for DefaultedUnknown<RichStrng> {
	fn from(t: String) -> Self {
		if t.is_empty() {
			DefaultedUnknown(None)
		} else {
			DefaultedUnknown(Some(t.into()))
		}
	}
}

impl From<Strng> for DefaultedUnknown<RichStrng> {
	fn from(t: Strng) -> Self {
		if t.is_empty() {
			DefaultedUnknown(None)
		} else {
			DefaultedUnknown(Some(t.into()))
		}
	}
}

impl From<Option<Strng>> for DefaultedUnknown<RichStrng> {
	fn from(t: Option<Strng>) -> Self {
		DefaultedUnknown(t.map(RichStrng::from))
	}
}

impl From<&Option<Strng>> for DefaultedUnknown<RichStrng> {
	fn from(t: &Option<Strng>) -> Self {
		DefaultedUnknown(t.as_ref().map(RichStrng::from))
	}
}

impl From<Option<&Strng>> for DefaultedUnknown<RichStrng> {
	fn from(t: Option<&Strng>) -> Self {
		DefaultedUnknown(t.map(RichStrng::from))
	}
}

impl<T> From<Option<T>> for DefaultedUnknown<T> {
	fn from(t: Option<T>) -> Self {
		DefaultedUnknown(t)
	}
}

impl<T> From<DefaultedUnknown<T>> for Option<T> {
	fn from(val: DefaultedUnknown<T>) -> Self {
		val.0
	}
}

impl<T: EncodeLabelValue> EncodeLabelValue for DefaultedUnknown<T> {
	fn encode(&self, writer: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
		match self {
			DefaultedUnknown(Some(i)) => i.encode(writer),
			DefaultedUnknown(None) => writer.write_str("unknown"),
		}
	}
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
// EncodeDisplay is a wrapper around a type that will be encoded with display
pub struct EncodeDisplay<T>(T);

impl<T: Display> EncodeLabelValue for EncodeDisplay<T> {
	fn encode(&self, writer: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
		writer.write_str(&self.0.to_string())
	}
}

impl<T: Display> From<T> for EncodeDisplay<T> {
	fn from(value: T) -> Self {
		EncodeDisplay(value)
	}
}

impl<T: Display> From<Option<T>> for DefaultedUnknown<EncodeDisplay<T>> {
	fn from(t: Option<T>) -> Self {
		DefaultedUnknown(t.map(EncodeDisplay::from))
	}
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
// EncodeDebug is a wrapper around a type that will be encoded with display
pub struct EncodeDebug<T>(T);

impl<T: Debug> EncodeLabelValue for EncodeDebug<T> {
	fn encode(&self, writer: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
		write!(writer, "{:?}", self.0)
	}
}

impl<T: Debug> From<T> for EncodeDebug<T> {
	fn from(value: T) -> Self {
		EncodeDebug(value)
	}
}

impl<T: Debug> From<Option<T>> for DefaultedUnknown<EncodeDebug<T>> {
	fn from(t: Option<T>) -> Self {
		DefaultedUnknown(t.map(EncodeDebug::from))
	}
}

#[derive(Default, Hash, PartialEq, Eq, Clone, Debug)]
// EncodeArc is a wrapper around a type to make Arc<T> encodable if T is
pub struct EncodeArc<T>(pub Arc<T>);

impl<T: EncodeLabelSet> EncodeLabelSet for EncodeArc<T> {
	fn encode(&self, encoder: &mut LabelSetEncoder) -> Result<(), Error> {
		self.0.encode(encoder)
	}
}

impl<T: EncodeLabelSet> From<Arc<T>> for EncodeArc<T> {
	fn from(value: Arc<T>) -> Self {
		EncodeArc(value)
	}
}

/// OptionallyEncode is a wrapper that will optionally encode the entire label set.
/// This differs from something like DefaultedUnknown which handles only the value - this makes the
/// entire label not show up.
#[derive(Clone, Hash, Default, Debug, PartialEq, Eq)]
pub struct OptionallyEncode<T>(Option<T>);

impl<T> From<Option<T>> for OptionallyEncode<T> {
	fn from(t: Option<T>) -> Self {
		OptionallyEncode(t)
	}
}

impl<T: EncodeLabelSet> EncodeLabelSet for OptionallyEncode<T> {
	fn encode(&self, encoder: &mut LabelSetEncoder) -> Result<(), std::fmt::Error> {
		match &self.0 {
			None => Ok(()),
			Some(ll) => ll.encode(encoder),
		}
	}
}

#[derive(Hash, PartialEq, Eq, Clone, Debug, Default)]
pub struct CustomField(Arc<[(RichStrng, DefaultedUnknown<RichStrng>)]>);

impl CustomField {
	pub fn new<K: Into<Strng>, V: Into<Strng>>(i: impl Iterator<Item = (K, Option<V>)>) -> Self {
		Self(
			i.into_iter()
				.map(|(k, v)| (RichStrng::from(k), DefaultedUnknown(v.map(RichStrng::from))))
				.collect(),
		)
	}
}

impl EncodeLabelSet for CustomField {
	fn encode(&self, encoder: &mut LabelSetEncoder) -> Result<(), Error> {
		self.0.as_ref().encode(encoder)
	}
}
