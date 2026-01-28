use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use hashbrown::Equivalent;

use crate::Value;
use crate::objects::StringValue;

#[derive(Debug, Clone)]
pub enum MapValue<'a> {
    Owned(Arc<hashbrown::HashMap<Key, Value<'static>>>),
    Borrow(vector_map::VecMap<KeyRef<'a>, Value<'a>>),
}

impl<'a> MapValue<'a> {
    pub fn iter_keys(&self) -> impl Iterator<Item = KeyRef<'a>> + use<'a, '_> {
        use itertools::Either;
        match self {
            MapValue::Owned(m) => Either::Left(m.keys().map(|k| KeyRef::from(k.clone()))),
            MapValue::Borrow(m) => Either::Right(m.keys().cloned()),
        }
    }
    pub fn iter(&'a self) -> impl Iterator<Item = (KeyRef<'a>, &'a Value<'a>)> {
        use itertools::Either;
        match self {
            MapValue::Owned(m) => Either::Left(m.iter().map(|(k, v)| (KeyRef::from(k), v))),
            MapValue::Borrow(m) => Either::Right(m.iter().map(|(k, v)| (k.clone(), v))),
        }
    }
    pub fn iter_owned(&'a self) -> impl Iterator<Item = (Key, Value<'static>)> + use<'a> {
        use itertools::Either;
        match self {
            MapValue::Owned(m) => Either::Left(m.iter().map(|(k, v)| (k.clone(), v.clone()))),
            MapValue::Borrow(m) => {
                Either::Right(m.iter().map(|(k, v)| (Key::from(k), v.as_static())))
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn len(&self) -> usize {
        match self {
            MapValue::Owned(m) => m.len(),
            MapValue::Borrow(m) => m.len(),
        }
    }
    pub fn contains_key(&self, key: &KeyRef) -> bool {
        match self {
            MapValue::Owned(m) => m.contains_key(key),
            MapValue::Borrow(m) => m.contains_key(key),
        }
    }
    fn get_raw<'r>(&'r self, key: &KeyRef<'r>) -> Option<&'r Value<'a>> {
        match self {
            MapValue::Owned(m) => m.get(key),
            MapValue::Borrow(m) => m.get(key),
        }
    }
    /// Returns a reference to the value corresponding to the key. Implicitly converts between int
    /// and uint keys.
    pub fn get<'r>(&'r self, key: &KeyRef<'r>) -> Option<&'r Value<'a>> {
        self.get_raw(key).or_else(|| match key {
            KeyRef::Int(k) => {
                let converted = u64::try_from(*k).ok()?;
                self.get_raw(&KeyRef::Uint(converted))
            }
            KeyRef::Uint(k) => {
                let converted = i64::try_from(*k).ok()?;
                self.get_raw(&KeyRef::Int(converted))
            }
            _ => None,
        })
    }
}

impl PartialOrd for MapValue<'_> {
    fn partial_cmp(&self, _: &Self) -> Option<Ordering> {
        None
    }
}

impl PartialEq for MapValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        // Compare lengths first for efficiency
        if self.len() != other.len() {
            return false;
        }
        // Compare all key-value pairs regardless of Owned/Borrow variant
        self.iter()
            .all(|(k, v)| other.get(&k).map_or(false, |other_v| v == other_v))
    }
}

impl<K: Into<Key>, V: Into<Value<'static>>> From<HashMap<K, V>> for MapValue<'static> {
    fn from(map: HashMap<K, V>) -> Self {
        let mut new_map = hashbrown::HashMap::with_capacity(map.len());
        for (k, v) in map {
            new_map.insert(k.into(), v.into());
        }
        MapValue::Owned(Arc::new(new_map))
    }
}

impl<K: Into<Key>, V: Into<Value<'static>>> From<hashbrown::HashMap<K, V>> for MapValue<'static> {
    fn from(map: hashbrown::HashMap<K, V>) -> Self {
        let mut new_map = hashbrown::HashMap::with_capacity(map.len());
        for (k, v) in map {
            new_map.insert(k.into(), v.into());
        }
        MapValue::Owned(Arc::new(new_map))
    }
}

impl<'a, K: Into<KeyRef<'a>>, V: Into<Value<'a>>> FromIterator<(K, V)> for Value<'a> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(map: T) -> Self {
        Value::Map(MapValue::Borrow(vector_map::VecMap::from_iter(
            map.into_iter().map(|(k, v)| (k.into(), v.into())),
        )))
    }
}

// Convert HashMap<K, V> to Value
impl<K: Into<Key>, V: Into<Value<'static>>> From<HashMap<K, V>> for Value<'static> {
    fn from(v: HashMap<K, V>) -> Self {
        Value::Map(v.into())
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Ord, Clone, PartialOrd)]
pub enum Key {
    Int(i64),
    Uint(u64),
    Bool(bool),
    String(Arc<str>),
}

/// Implement conversions from primitive types to [`Key`]
impl From<bool> for Key {
    fn from(v: bool) -> Self {
        Key::Bool(v)
    }
}

impl From<i64> for Key {
    fn from(v: i64) -> Self {
        Key::Int(v)
    }
}

impl From<i32> for Key {
    fn from(v: i32) -> Self {
        Key::Int(v as i64)
    }
}

impl From<u64> for Key {
    fn from(v: u64) -> Self {
        Key::Uint(v)
    }
}

impl From<u32> for Key {
    fn from(v: u32) -> Self {
        Key::Uint(v as u64)
    }
}

impl From<&str> for Key {
    fn from(v: &str) -> Self {
        Key::String(Arc::from(v))
    }
}

impl From<String> for Key {
    fn from(v: String) -> Self {
        Key::String(Arc::from(v.as_str()))
    }
}

impl<'a> PartialEq<KeyRef<'a>> for Key {
    fn eq(&self, key: &KeyRef) -> bool {
        &KeyRef::from(self) == key
    }
}

impl serde::Serialize for Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Key::Int(v) => v.serialize(serializer),
            Key::Uint(v) => v.serialize(serializer),
            Key::Bool(v) => v.serialize(serializer),
            Key::String(v) => v.serialize(serializer),
        }
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::Int(v) => write!(f, "{v}"),
            Key::Uint(v) => write!(f, "{v}"),
            Key::Bool(v) => write!(f, "{v}"),
            Key::String(v) => write!(f, "{v}"),
        }
    }
}

/// Implement conversions from [`Key`] into [`Value`]
impl<'a> TryInto<Key> for Value<'a> {
    type Error = Value<'static>;

    #[inline(always)]
    fn try_into(self) -> Result<Key, Self::Error> {
        match self {
            Value::Int(v) => Ok(Key::Int(v)),
            Value::UInt(v) => Ok(Key::Uint(v)),
            Value::String(v) => Ok(Key::String(v.as_owned())),
            Value::Bool(v) => Ok(Key::Bool(v)),
            _ => Err(self.as_static()),
        }
    }
}

/// A borrowed version of [`Key`] that avoids allocating for lookups.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum KeyRef<'a> {
    Int(i64),
    Uint(u64),
    Bool(bool),
    String(StringValue<'a>),
}

impl Equivalent<Key> for KeyRef<'_> {
    fn equivalent(&self, key: &Key) -> bool {
        self == &KeyRef::from(key)
    }
}

impl<'a> From<&KeyRef<'a>> for Key {
    fn from(value: &KeyRef<'a>) -> Self {
        match value {
            KeyRef::Int(v) => Key::Int(*v),
            KeyRef::Uint(v) => Key::Uint(*v),
            KeyRef::Bool(v) => Key::Bool(*v),
            KeyRef::String(v) => Key::String(v.as_owned()),
        }
    }
}
impl<'a> From<KeyRef<'a>> for Value<'a> {
    fn from(value: KeyRef<'a>) -> Self {
        match value {
            KeyRef::Int(v) => Value::Int(v),
            KeyRef::Uint(v) => Value::UInt(v),
            KeyRef::Bool(v) => Value::Bool(v),
            KeyRef::String(v) => Value::String(v),
        }
    }
}
impl Display for KeyRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyRef::Int(v) => write!(f, "{v}"),
            KeyRef::Uint(v) => write!(f, "{v}"),
            KeyRef::Bool(v) => write!(f, "{v}"),
            KeyRef::String(v) => f.write_str(v.as_ref()),
        }
    }
}

impl<'a> From<&'a Key> for KeyRef<'a> {
    fn from(value: &'a Key) -> Self {
        match value {
            Key::Int(v) => KeyRef::Int(*v),
            Key::Uint(v) => KeyRef::Uint(*v),
            Key::String(v) => KeyRef::String(StringValue::Borrowed(v.as_ref())),
            Key::Bool(v) => KeyRef::Bool(*v),
        }
    }
}

impl<'a> From<&'a str> for KeyRef<'a> {
    fn from(value: &'a str) -> Self {
        KeyRef::String(StringValue::Borrowed(value))
    }
}

impl<'a> From<&'a String> for KeyRef<'a> {
    fn from(value: &'a String) -> Self {
        KeyRef::String(StringValue::Borrowed(value))
    }
}

impl From<Key> for KeyRef<'static> {
    fn from(value: Key) -> Self {
        match value {
            Key::Int(v) => KeyRef::Int(v),
            Key::Uint(v) => KeyRef::Uint(v),
            Key::String(v) => KeyRef::String(StringValue::Owned(v.clone())),
            Key::Bool(v) => KeyRef::Bool(v),
        }
    }
}
/// Implement conversions from [`KeyRef`] into [`Value`]
impl<'a> TryFrom<&'a Value<'a>> for KeyRef<'a> {
    type Error = Value<'a>;

    fn try_from(value: &'a Value) -> Result<Self, Self::Error> {
        match value {
            Value::Int(v) => Ok(KeyRef::Int(*v)),
            Value::UInt(v) => Ok(KeyRef::Uint(*v)),
            Value::String(v) => Ok(KeyRef::String(v.as_ref().into())),
            Value::Bool(v) => Ok(KeyRef::Bool(*v)),
            _ => Err(value.clone()),
        }
    }
}
