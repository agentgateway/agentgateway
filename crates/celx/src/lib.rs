use cel::Context;

mod cidr;
mod general;
mod strings;
#[cfg(test)]
#[path = "function_tests.rs"]
mod tests;

pub use general::FlattenSignal;

pub fn insert_all(ctx: &mut Context<'_>) {
	// General agentgateway additional functions
	general::insert_all(ctx);
	// "Strings" extension
	// https://pkg.go.dev/github.com/google/cel-go/ext#Strings
	strings::insert_all(ctx);
	// https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-cidr-library and
	// https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-ip-address-library
	cidr::insert_all(ctx);
}

mod helpers {
	pub trait TypeName {
		fn type_name() -> &'static str;
	}

	use std::sync::Arc;

	use cel::extractors::{IntoResolveResult, This};
	use cel::objects::{Opaque, ValueType};
	use cel::{ExecutionError, FunctionContext, ResolveResult, Value};

	pub type FResult<T> = std::result::Result<T, ExecutionError>;
	pub type FVResult = std::result::Result<Value, ExecutionError>;

	pub fn cast<T: Opaque + TypeName>(val: &Arc<dyn Opaque>) -> FResult<&T> {
		val
			.downcast_ref::<T>()
			.ok_or_else(|| ExecutionError::UnexpectedType {
				got: val.runtime_type_name().to_string(),
				want: <T as TypeName>::type_name().to_string(),
			})
	}

	#[macro_export]
	macro_rules! impl_opaque {
		($type:ty, $name:literal) => {
			impl $crate::helpers::TypeName for $type {
				#[inline]
				fn type_name() -> &'static str {
					$name
				}
			}
			impl cel::objects::Opaque for $type {
				#[inline]
				fn runtime_type_name(&self) -> &str {
					<Self as $crate::helpers::TypeName>::type_name()
				}

				fn json(&self) -> Option<serde_json::Value> {
					serde_json::to_value(&self).ok()
				}
			}
		};
	}

	pub type Function = Box<dyn Fn(&mut FunctionContext) -> ResolveResult + Send + Sync>;

	pub fn wrap1<F, T, V>(f: F) -> Function
	where
		F: Fn(&T) -> V + Send + Sync + 'static,
		V: cel::extractors::IntoResolveResult,
		T: Opaque + TypeName + 'static + Send + Sync,
	{
		let closure = move |This(this): This<Value>| {
			let this = as_opaque(this)?;
			f(cast::<T>(&this)?).into_resolve_result()
		};
		cel::extractors::IntoFunction::into_function(closure)
	}
	pub fn wrap2<F, T, V, A1>(f: F) -> Function
	where
		F: Fn(&T, &A1) -> FResult<V> + Send + Sync + 'static,
		V: cel::extractors::IntoResolveResult,
		T: Opaque + TypeName + 'static + Send + Sync,
		A1: Opaque + TypeName + 'static + Send + Sync,
	{
		let closure = move |This(this): This<Value>, a1: Value| {
			let this = as_opaque(this)?;
			let a1 = as_opaque(a1)?;
			f(cast::<T>(&this)?, cast::<A1>(&a1)?)?.into_resolve_result()
		};
		cel::extractors::IntoFunction::into_function(closure)
	}
	pub fn wrap2_val<F, T, V>(f: F) -> Function
	where
		F: Fn(&T, &FunctionContext, Value) -> FResult<V> + Send + Sync + 'static,
		V: cel::extractors::IntoResolveResult,
		T: Opaque + TypeName + 'static + Send + Sync,
	{
		let closure = move |ftx: &FunctionContext, This(this): This<Value>, a1: Value| {
			let this = as_opaque(this)?;
			f(cast::<T>(&this)?, ftx, a1)?.into_resolve_result()
		};
		cel::extractors::IntoFunction::into_function(closure)
	}
	pub fn wrapnew<F, V>(f: F) -> Function
	where
		F: Fn(&FunctionContext, &str) -> FResult<V> + Send + Sync + 'static,
		V: Opaque,
	{
		let closure = move |ftx: &FunctionContext, a1: Value| {
			let Value::String(a) = a1 else {
				return Err(ExecutionError::UnexpectedType {
					got: a1.type_of().to_string(),
					want: ValueType::String.to_string(),
				});
			};
			let res = f(ftx, &a)?;
			Ok(Value::Opaque(Arc::new(res))).into_resolve_result()
		};
		cel::extractors::IntoFunction::into_function(closure)
	}

	pub fn split_this(this: Function, not_this: Function) -> Function {
		let closure = move |ftx: &mut FunctionContext| {
			if ftx.this.is_some() {
				this(ftx)
			} else {
				not_this(ftx)
			}
		};
		Box::new(closure)
	}

	fn as_opaque(a: Value) -> Result<Arc<dyn Opaque>, ExecutionError> {
		let Value::Opaque(a) = a else {
			return Err(ExecutionError::UnexpectedType {
				got: a.type_of().to_string(),
				want: ValueType::Opaque.to_string(),
			});
		};
		Ok(a)
	}
}
