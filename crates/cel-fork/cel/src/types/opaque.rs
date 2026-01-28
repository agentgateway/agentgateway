use std::any::Any;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::functions::FunctionContext;
use crate::objects::ResolveResult;

/// Equality helper for [`Opaque`] values.
///
/// Implementors define how two values of the same runtime type compare for
/// equality when stored as [`Value::Opaque`].
///
/// You normally don't implement this trait manually. It is automatically
/// provided for any `T: Eq + PartialEq + Any + Opaque` (see the blanket impl
/// below). The runtime will first ensure the two values have the same
/// [`Opaque::runtime_type_name`], and only then attempt a downcast and call
/// `Eq::eq`.
pub trait OpaqueEq {
    /// Compare with another [`Opaque`] erased value.
    ///
    /// Implementations should return `false` if `other` does not have the same
    /// runtime type, or if it cannot be downcast to the concrete type of `self`.
    fn opaque_eq(&self, other: &dyn Opaque) -> bool;
}

impl<T> OpaqueEq for T
where
    T: Eq + PartialEq + Any + Opaque,
{
    fn opaque_eq(&self, other: &dyn Opaque) -> bool {
        if self.type_name() != other.type_name() {
            return false;
        }
        if let Some(other) = other.downcast_ref::<T>() {
            self.eq(other)
        } else {
            false
        }
    }
}

/// Helper trait to obtain a `&dyn Debug` view.
///
/// This is auto-implemented for any `T: Debug` and is used by the runtime to
/// format [`Opaque`] values without knowing their concrete type.
pub trait AsDebug {
    /// Returns `self` as a `&dyn Debug` trait object.
    fn as_debug(&self) -> &dyn Debug;
}

impl<T> AsDebug for T
where
    T: Debug,
{
    fn as_debug(&self) -> &dyn Debug {
        self
    }
}

pub trait Opaque: Any + OpaqueEq + AsDebug + Send + Sync + erased_serde::Serialize {
    /// Returns a stable, fully-qualified type name for this value's runtime type.
    ///
    /// This name is used to check type compatibility before attempting downcasts
    /// during equality checks and other operations. It should be stable across
    /// versions and unique within your application or library (e.g., a package
    /// qualified name like `my.pkg.Type`).
    #[inline]
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Resolves a method function by name.
    fn call_function<'a, 'rf>(
        &self,
        _name: &str,
        _ftx: &mut FunctionContext<'a, 'rf>,
    ) -> Option<ResolveResult<'a>> {
        None
    }
}

impl dyn Opaque {
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        let any: &dyn Any = self;
        any.downcast_ref()
    }
}

/// A covariant Arc-based container for user-defined object values.
///
/// This type stores values implementing [`Opaque`] using Arc for cheap cloning.
/// It is covariant in 'a because:
/// - The actual data pointer doesn't carry lifetime info (it's erased)
/// - PhantomData<fn() -> Value<'a>> is covariant in 'a (function return types are covariant)
///
/// # Example
/// ```rust
/// use cel::objects::{OpaqueValue, Opaque, Value};
/// use serde::Serialize;
///
/// #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
/// struct MyStruct { field: String }
///
/// impl Opaque for MyStruct {
///     fn type_name(&self) -> &'static str { "MyStruct" }
/// }
///
/// let obj = OpaqueValue::new(MyStruct { field: "test".into() });
/// let value: Value = obj.into();
/// ```
#[derive(Clone)]
pub struct OpaqueValue(Arc<dyn Opaque>);

impl Debug for OpaqueValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.as_debug().fmt(f)
    }
}
impl PartialEq for OpaqueValue {
    fn eq(&self, other: &Self) -> bool {
        self.0.opaque_eq(other.0.as_ref())
    }
}

impl OpaqueValue {
    /// Create a new Object from a value implementing ObjectValue
    pub fn new<T>(value: T) -> Self
    where
        T: Opaque,
    {
        let arc = Arc::new(value);
        OpaqueValue(arc)
    }

    /// Returns the type name of the contained value.
    pub fn type_name(&self) -> &'static str {
        self.0.type_name()
    }

    /// Resolves a method function by name.
    pub fn call_function<'a, 'rf>(
        &self,
        name: &str,
        ftx: &mut FunctionContext<'a, 'rf>,
    ) -> Option<ResolveResult<'a>> {
        self.0.call_function(name, ftx)
    }

    /// Attempts to downcast to a concrete type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Returns the JSON representation if available.
    pub fn json(&self) -> Option<serde_json::Value> {
        erased_serde::serialize(&*self.0, serde_json::value::Serializer).ok()
    }
}
