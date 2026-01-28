#[macro_export]
macro_rules! impl_conversions {
    // Capture pairs separated by commas, where each pair is separated by =>
    ($($target_type:ty => $value_variant:path),* $(,)?) => {
        $(
            impl<'a> FromValue<'a> for $target_type {
                fn from_value(expr: &Value<'a>) -> Result<Self, ExecutionError> {
                    if let $value_variant(v) = expr {
                        Ok(v.clone())
                    } else if let Value::Dynamic(d) = expr {
                        // Try to materialize and extract
                        let materialized = d.materialize();
                        if let $value_variant(v) = materialized {
                            Ok(v.clone())
                        } else {
                            Err(ExecutionError::UnexpectedType {
                                got: materialized.type_of().as_str(),
                                want: stringify!($target_type),
                            })
                        }
                    } else {
                        Err(ExecutionError::UnexpectedType {
                            got: expr.type_of().as_str(),
                            want: stringify!($target_type),
                        })
                    }
                }
            }

            impl<'a> FromValue<'a> for Option<$target_type> {
                fn from_value(expr: &Value<'a>) -> Result<Self, ExecutionError> {
                    match expr {
                        Value::Null => Ok(None),
                        $value_variant(v) => Ok(Some(v.clone())),
                        Value::Dynamic(d) => {
                            let materialized = d.materialize();
                            match materialized {
                                Value::Null => Ok(None),
                                $value_variant(v) => Ok(Some(v.clone())),
                                _ => Err(ExecutionError::UnexpectedType {
                                    got: materialized.type_of().as_str(),
                                    want: stringify!($target_type),
                                }),
                            }
                        }
                        _ => Err(ExecutionError::UnexpectedType {
                            got: expr.type_of().as_str(),
                            want: stringify!($target_type),
                        }),
                    }
                }
            }

            impl<'a> From<$target_type> for Value<'a> {
                fn from(value: $target_type) -> Self {
                    $value_variant(value.into())
                }
            }
        )*
    }
}

#[macro_export]
macro_rules! impl_handler {
    ($($t:ty),*) => {
        paste::paste! {
            impl<F, $($t,)*> IntoFunction<(WithFunctionContext, $($t,)*)> for F
            where
                F: for <'a, 'rf> Fn(&mut FunctionContext<'a, 'rf>, $($t,)*) -> ResolveResult<'a> + Send + Sync + 'static,
                $($t: for<'a, 'rf> $crate::FromContext<'a, 'rf>,)*
            {
                fn into_function(self) -> Function {
                    Box::new(move |mut _ftx| {
                        $(
                            let [<arg_ $t:lower>] = $t::from_context(&mut _ftx);
                        )*
                        self(_ftx, $([<arg_ $t:lower>],)*).into()
                    })
                }
            }
        }
    };
}

pub(crate) use {impl_conversions, impl_handler};
