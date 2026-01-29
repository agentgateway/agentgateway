use cel::Context;

mod cidr;
mod flatten;
mod general;
mod optimize;
mod strings;
#[cfg(test)]
#[path = "function_tests.rs"]
mod tests;

pub use flatten::FlattenSignal;
pub use optimize::DefaultOptimizer;

pub fn insert_all(ctx: &mut Context) {
	// General agentgateway additional functions
	general::insert_all(ctx);
	// "Strings" extension
	// https://pkg.go.dev/github.com/google/cel-go/ext#Strings
	strings::insert_all(ctx);
	// https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-cidr-library and
	// https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-ip-address-library
	cidr::insert_all(ctx);
	// Optimized functions
	optimize::insert_all(ctx);
	flatten::insert_all(ctx);
}

mod helpers {
	use cel::extractors::Function;
	use cel::objects::{Opaque, OpaqueValue, StringValue};
	use cel::{ExecutionError, FunctionContext, Value};

	pub type FResult<T> = Result<T, ExecutionError>;
	pub type FVResult<'a> = Result<Value<'a>, ExecutionError>;

	pub trait TypeName {
		fn type_name() -> &'static str;
	}

	pub fn cast<T: Opaque + TypeName>(val: &OpaqueValue) -> FResult<&T> {
		val
			.downcast_ref::<T>()
			.ok_or_else(|| ExecutionError::UnexpectedType {
				got: val.type_name(),
				want: <T as TypeName>::type_name(),
			})
	}

	#[macro_export]
	macro_rules! impl_functions {
    ({$($basic:tt => $name:literal),* $(,)?}, {$($full:tt => $fname:literal),* $(,)?}) => {
				#[allow(unused_variables)]
        fn call_function<'rr, 'rrf>(&self, name: &str, ftx: &mut FunctionContext<'rr, 'rrf>) -> Option<cel::ResolveResult<'rr>> {
            match name {
                $(
                    $name => Some(self.$basic().into()),
                )*
                $(
                    $fname => Some(self.$full(ftx).into()),
                )*
                _ => None,
            }
        }
    };
    // Helper: handle ident => "name" form
    (@match_arm $method:ident => $name:literal, $ftx:ident) => {
        $name => Some(self.$method($ftx)),
    };

    // Helper: handle plain ident form
    (@match_arm $method:ident, $ftx:ident) => {
        stringify!($method) => Some(self.$method($ftx)),
    };
}
	#[macro_export]
	macro_rules! impl_opaque {
		($type:ty, $name:literal) => {
			impl<'a> $crate::helpers::TypeName for $type {
				#[inline]
				fn type_name() -> &'static str {
					std::any::type_name::<Self>()
				}
			}
			impl cel::objects::Opaque for $type {
				fn call_function<'a, 'rf>(
					&self,
					name: &str,
					ftx: &mut FunctionContext<'a, 'rf>,
				) -> Option<cel::ResolveResult<'a>> {
					self.call_function(name, ftx)
				}
			}
		};
	}

	fn funnel<CL>(f: CL) -> CL
	where
		CL: for<'b, 'a, 'rf> Fn(&'b mut FunctionContext<'a, 'rf>) -> FVResult<'a>,
	{
		f
	}

	pub fn wrapnew<F, V>(f: F) -> Function
	where
		F: Fn(&FunctionContext, &str) -> FResult<V> + Send + Sync + 'static,
		V: Opaque + TypeName + Clone + PartialEq,
	{
		let closure = funnel(move |ftx: &mut FunctionContext| {
			let a1: StringValue = ftx.arg(0)?;
			let res = f(ftx, a1.as_ref())?;
			Ok(Value::Object(OpaqueValue::new(res)))
		});
		cel::extractors::IntoFunction::into_function(closure)
	}
}
