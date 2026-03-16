use std::fmt::Debug;
use std::net::IpAddr;
use std::sync::Arc;

use crate::Value;
use crate::functions::FunctionContext;
use crate::objects::StringValue;
use cel::objects::KeyRef;
use cel::{to_value, types};
use vector_map::VecMap;

pub fn maybe_materialize_optional<T: DynamicType>(t: &Option<T>) -> Value<'_> {
	match t {
		Some(v) => maybe_materialize(v),
		None => Value::Null,
	}
}
pub fn maybe_materialize<T: DynamicType>(t: &T) -> Value<'_> {
	if t.auto_materialize() {
		t.materialize()
	} else {
		Value::Dynamic(DynamicValue::new(t))
	}
}

pub trait DynamicType: std::fmt::Debug + Send + Sync {
	// If the value can be freely converted to a Value, do so.
	// This is anything but list/map
	fn auto_materialize(&self) -> bool {
		false
	}

	// Convert this dynamic value into a proper value
	fn materialize(&self) -> Value<'_>;

	#[allow(unused_variables)]
	fn field(&self, field: &str) -> Option<Value<'_>> {
		None
	}

	/// Resolves a method function by name.
	#[allow(unused_variables)]
	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<crate::ResolveResult<'a>>
	where
		Self: 'a,
	{
		None
	}
}

/// Trait for types that can be flattened into a parent struct's map.
/// This is automatically implemented by the DynamicType derive macro for structs,
/// and manually implemented for map-like types.
pub trait DynamicFlatten: DynamicType {
	/// Insert this type's fields directly into the given map.
	fn materialize_into<'a>(
		&'a self,
		map: &mut vector_map::VecMap<crate::objects::KeyRef<'a>, Value<'a>>,
	);
}

#[derive(Clone)]
enum DynamicRef<'a> {
	Borrowed(&'a dyn DynamicType),
	Owned(Arc<dyn DynamicType + 'a>),
}

pub struct DynamicValue<'a> {
	dyn_ref: DynamicRef<'a>,
}

impl<'a> DynamicValue<'a> {
	pub fn new<T: DynamicType>(t: &'a T) -> Self {
		Self {
			dyn_ref: DynamicRef::Borrowed(t as &dyn DynamicType),
		}
	}

	pub fn new_owned<T: DynamicType + 'a>(t: T) -> Self {
		Self {
			dyn_ref: DynamicRef::Owned(Arc::new(t)),
		}
	}

	fn as_ref(&self) -> &(dyn DynamicType + 'a) {
		match &self.dyn_ref {
			DynamicRef::Borrowed(dyn_ref) => *dyn_ref,
			DynamicRef::Owned(dyn_ref) => dyn_ref.as_ref(),
		}
	}

	pub fn materialize(&self) -> Value<'a> {
		match &self.dyn_ref {
			DynamicRef::Borrowed(dyn_ref) => dyn_ref.materialize(),
			DynamicRef::Owned(dyn_ref) => dyn_ref.materialize().as_static(),
		}
	}

	pub fn field(&self, field: &str) -> Option<Value<'a>> {
		match &self.dyn_ref {
			DynamicRef::Borrowed(dyn_ref) => dyn_ref.field(field),
			DynamicRef::Owned(dyn_ref) => dyn_ref.field(field).map(|value| value.as_static()),
		}
	}

	pub fn call_function<'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<crate::ResolveResult<'a>> {
		match &self.dyn_ref {
			DynamicRef::Borrowed(dyn_ref) => dyn_ref.call_function(name, ftx),
			DynamicRef::Owned(dyn_ref) => dyn_ref.call_function(name, ftx),
		}
	}
}

impl<'a> Clone for DynamicValue<'a> {
	fn clone(&self) -> Self {
		Self {
			dyn_ref: self.dyn_ref.clone(),
		}
	}
}

impl<'a> std::fmt::Debug for DynamicValue<'a> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.as_ref().fmt(f)
	}
}

impl DynamicType for Value<'_> {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		self.clone()
	}
}

// Primitive type implementations

// &str - auto-materializes to String value
impl DynamicType for &str {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self)
	}
}
impl DynamicType for arcstr::ArcStr {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		// TODO: do we want to clone or reference? For now, reference...
		Value::from(self.as_str())
	}
}

// String - auto-materializes to String value
impl DynamicType for String {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(self.as_str())
	}
}

// bool - auto-materializes to Bool value
impl DynamicType for bool {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self)
	}
}

// i64 - auto-materializes to Int value
impl DynamicType for i64 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self)
	}
}

// u64 - auto-materializes to Int value (as i64)
impl DynamicType for u64 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self)
	}
}

// i32 - auto-materializes to Int value
impl DynamicType for i32 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self as i64)
	}
}

// u32 - auto-materializes to Int value
impl DynamicType for u32 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self as u64)
	}
}
impl DynamicType for u16 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self as u64)
	}
}
impl DynamicType for u8 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self as u64)
	}
}

// f64 - auto-materializes to Float value
impl DynamicType for f64 {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(*self)
	}
}
impl DynamicType for IpAddr {
	fn auto_materialize(&self) -> bool {
		true
	}

	fn materialize(&self) -> Value<'_> {
		Value::from(self.to_string())
	}
}

// Collection types - these do NOT auto-materialize since they're complex structures

// HashMap<String, String> - materializes to Map value
impl DynamicType for std::collections::HashMap<String, String> {
	fn materialize(&self) -> Value<'_> {
		let mut map = vector_map::VecMap::with_capacity(self.len());
		for (k, v) in self.iter() {
			map.insert(
				crate::objects::KeyRef::from(k.as_str()),
				Value::from(v.as_str()),
			);
		}
		Value::Map(crate::objects::MapValue::Borrow(map))
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		self.get(field).map(|v| Value::from(v.as_str()))
	}
}

impl DynamicFlatten for std::collections::HashMap<String, String> {
	fn materialize_into<'a>(
		&'a self,
		map: &mut vector_map::VecMap<crate::objects::KeyRef<'a>, Value<'a>>,
	) {
		for (k, v) in self.iter() {
			map.insert(
				crate::objects::KeyRef::from(k.as_str()),
				Value::from(v.as_str()),
			);
		}
	}
}

impl<T: DynamicType> DynamicType for &T {
	fn auto_materialize(&self) -> bool {
		(*self).auto_materialize()
	}

	fn materialize(&self) -> Value<'_> {
		(*self).materialize()
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		(*self).field(field)
	}

	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<crate::ResolveResult<'a>>
	where
		Self: 'a,
	{
		(*self).call_function(name, ftx)
	}
}

// Vec<T> - materializes to List value
impl<T> DynamicType for Vec<T>
where
	T: Debug + DynamicType,
{
	fn materialize<'a>(&'a self) -> Value<'a> {
		let items: Vec<Value<'a>> = self.iter().map(|s| s.materialize()).collect();
		Value::List(crate::objects::ListValue::PartiallyOwned(items.into()))
	}
}
impl<T> DynamicType for Arc<T>
where
	T: Debug + DynamicType,
{
	fn auto_materialize(&self) -> bool {
		self.as_ref().auto_materialize()
	}
	fn materialize<'a>(&'a self) -> Value<'a> {
		self.as_ref().materialize()
	}
	fn field(&self, field: &str) -> Option<Value<'_>> {
		self.as_ref().field(field)
	}
	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<crate::ResolveResult<'a>>
	where
		Self: 'a,
	{
		self.as_ref().call_function(name, ftx)
	}
}
impl<T> DynamicType for Option<T>
where
	T: Debug + DynamicType,
{
	fn auto_materialize(&self) -> bool {
		match self {
			Some(v) => v.auto_materialize(),
			None => true,
		}
	}
	fn materialize<'a>(&'a self) -> Value<'a> {
		match self {
			Some(v) => v.materialize(),
			None => Value::Null,
		}
	}
	fn field(&self, field: &str) -> Option<Value<'_>> {
		match self {
			Some(v) => v.field(field),
			None => None,
		}
	}
	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<crate::ResolveResult<'a>>
	where
		Self: 'a,
	{
		match self {
			Some(v) => v.call_function(name, ftx),
			None => None,
		}
	}
}
impl<T> DynamicFlatten for Option<T>
where
	T: Debug + DynamicFlatten,
{
	fn materialize_into<'a>(&'a self, map: &mut VecMap<KeyRef<'a>, Value<'a>>) {
		if let Some(v) = self {
			v.materialize_into(map);
		}
	}
}

impl DynamicType for serde_json::Value {
	fn auto_materialize(&self) -> bool {
		!matches!(
			self,
			serde_json::Value::Array(_) | serde_json::Value::Object(_)
		)
	}
	fn materialize(&self) -> Value<'_> {
		match self {
			serde_json::Value::Null => Value::Null,
			serde_json::Value::Bool(v) => Value::Bool(*v),
			serde_json::Value::Number(v) => {
				if v.is_i64() {
					Value::Int(v.as_i64().unwrap())
				} else if v.is_u64() {
					Value::UInt(v.as_u64().unwrap())
				} else {
					Value::Float(v.as_f64().unwrap())
				}
			},
			serde_json::Value::String(v) => Value::String(StringValue::Borrowed(v.as_str())),
			serde_json::Value::Array(v) => to_value(v).unwrap(),
			serde_json::Value::Object(v) => to_value(v).unwrap(),
		}
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		match self {
			serde_json::Value::Object(m) => {
				let v = m.get(field)?;
				Some(types::dynamic::maybe_materialize(v))
			},
			_ => None,
		}
	}
}

impl DynamicFlatten for serde_json::Value {
	fn materialize_into<'a>(
		&'a self,
		map: &mut vector_map::VecMap<crate::objects::KeyRef<'a>, Value<'a>>,
	) {
		if let serde_json::Value::Object(obj) = self {
			for (k, v) in obj.iter() {
				map.insert(
					crate::objects::KeyRef::from(k.as_str()),
					maybe_materialize(v),
				);
			}
		}
	}
}

impl DynamicType for serde_json::Map<String, serde_json::Value> {
	fn materialize(&self) -> Value<'_> {
		to_value(self).unwrap()
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		let v = self.get(field)?;
		Some(types::dynamic::maybe_materialize(v))
	}
}

impl DynamicFlatten for serde_json::Map<String, serde_json::Value> {
	fn materialize_into<'a>(
		&'a self,
		map: &mut vector_map::VecMap<crate::objects::KeyRef<'a>, Value<'a>>,
	) {
		for (k, v) in self.iter() {
			map.insert(
				crate::objects::KeyRef::from(k.as_str()),
				maybe_materialize(v),
			);
		}
	}
}
