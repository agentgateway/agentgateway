//! Derive macros for cel-rust

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, FieldsNamed, Lit, Meta, MetaNameValue, Variant,
    parse_macro_input,
};

/// Derive `DynamicType` for a struct, tuple struct, or enum.
///
/// # Supported Types
///
/// - **Named structs**: Structs with named fields (e.g., `struct Foo { field: T }`)
/// - **Newtype tuple structs**: Single-field tuple structs (e.g., `struct Wrapper(T)`)
/// - **Unit enums**: Enums with only unit variants (e.g., `enum Status { Active, Inactive }`)
///
/// # Attributes
///
/// ## Struct/Enum-level attributes
///
/// - `#[dynamic(crate = "path")]` - Specify the path to the cel crate (default: `::cel`)
///   - Use `#[dynamic(crate = "crate")]` when using this derive inside the cel crate itself
///   - Use `#[dynamic(crate = "::cel")]` or omit for external usage
/// - `#[dynamic(rename_all = "case")]` - Apply a naming convention to all fields
///   - Supported cases: `"camelCase"`, `"lowercase"`
///   - Example: `#[dynamic(rename_all = "camelCase")]` transforms `user_name` to `userName`
///
/// ## Field-level attributes (Named Structs Only)
///
/// - `#[dynamic(skip)]` - Skip this field in the generated implementation
/// - `#[dynamic(rename = "name")]` - Use a different name for this field in CEL
///
/// - `#[dynamic(skip_serializing_if = "function")]` - Skip this field if the predicate returns true.
///   The function receives `&self.field` and should return a `bool`. When the predicate returns `true`,
///   the field is not included in the materialized map (not even as `null`). When accessing the field
///   via `.field("name")`, it returns `None` if the predicate is true.
///   
///   Example:
///   ```rust,ignore
///   #[derive(DynamicType)]
///   pub struct HttpRequest<'a> {
///       method: &'a str,
///       #[dynamic(skip_serializing_if = "Option::is_none")]
///       claims: Option<&'a Claims>,
///   }
///   // When claims is None, it's not included in the materialized map at all
///   ```
///
/// ## Serde Compatibility
///
/// The derive macro also reads serde attributes when present:
/// - `#[serde(skip)]` - Same as `#[dynamic(skip)]`
/// - `#[serde(rename = "name")]` - Same as `#[dynamic(rename = "name")]`
/// - `#[serde(rename_all = "case")]` - Same as `#[dynamic(rename_all = "case")]`
/// - `#[serde(skip_serializing_if = "function")]` - Same as `#[dynamic(skip_serializing_if = "function")]`
///
/// When both `#[dynamic(...)]` and `#[serde(...)]` attributes are present on the same
/// field or struct, the `#[dynamic(...)]` attribute takes precedence
/// - `#[dynamic(flatten)]` - Flatten the contents of this field into the parent struct.
///   The field must implement `DynamicFlatten`. When materializing, the field's contents are
///   merged into the parent map instead of being nested. When accessing fields, lookups
///   will fall through to the flattened field if not found in the parent.
///   
///   `DynamicFlatten` is implemented for:
///   - Structs with `#[derive(DynamicType)]`
///   - `serde_json::Value` and `serde_json::Map<String, serde_json::Value>`
///   - `std::collections::HashMap<String, String>`
///   - `http::HeaderMap`
///   
///   Example:
///   ```rust,ignore
///   #[derive(DynamicType)]
///   pub struct Claims {
///       pub key: String,
///       #[dynamic(flatten)]
///       pub metadata: serde_json::Value,
///   }
///   // Accessing: claims.foo will look up metadata.field("foo") if "foo" is not a direct field
///   ```
///
/// ## Variant-level attributes (Unit Enums Only)
///
/// - `#[dynamic(rename = "name")]` - Use a different string value for this variant
///   
///   Example:
///   ```rust,ignore
///   #[derive(DynamicType)]
///   pub enum Protocol {
///       #[dynamic(rename = "http")]
///       Http,
///       #[dynamic(rename = "grpc")]
///       Grpc,
///   }
///   // Protocol::Http materializes to Value::String("http")
///   ```
///
/// - `#[dynamic(with = "function")]` - Transform the field value using a helper function before
///   passing to `maybe_materialize`. The function receives `&self.field` (note: if the field
///   is already a reference like `&'a T`, the function receives `&&'a T`) and should return
///   a reference to something that implements `DynamicType`.
///   
///   **Important**: Due to type inference limitations, you should use a named helper function
///   with explicit lifetime annotations rather than inline closures.
///   
///   Example:
///   ```rust,ignore
///   // Define a helper function with explicit lifetimes
///   fn extract_claims<'a>(c: &'a &'a Claims) -> &'a serde_json::Value {
///       &c.0
///   }
///   
///   #[derive(DynamicType)]
///   pub struct HttpRequest<'a> {
///       #[dynamic(with = "extract_claims")]
///       claims: &'a Claims,
///   }
///   ```
///
/// - `#[dynamic(with_value = "function")]` - Transform the field value using a helper function
///   that returns a `Value` directly. The function receives `&self.field` and must return `Value<'_>`.
///   This is useful for types that implement `AsRef<str>` or other conversions.
///   
///   Example:
///   ```rust,ignore
///   fn method_to_value<'a, T: AsRef<str>>(c: &'a &'a T) -> Value<'a> {
///       Value::String(c.as_ref().into())
///   }
///   
///   #[derive(DynamicType)]
///   pub struct HttpRequest<'a> {
///       #[dynamic(with_value = "method_to_value")]
///       method: &'a http::Method,
///   }
///   ```
///
/// ```rust,ignore
/// use cel::DynamicType;
///
/// // Named struct
/// #[derive(DynamicType)]
/// pub struct HttpRequest<'a> {
///     method: &'a str,
///     path: &'a str,
///     #[dynamic(skip)]
///     internal_id: u64,
/// }
///
/// // Newtype tuple struct (wraps another DynamicType)
/// #[derive(DynamicType)]
/// pub struct Metadata(serde_json::Map<String, serde_json::Value>);
///
/// // Unit enum (materializes to string)
/// #[derive(DynamicType)]
/// pub enum Protocol {
///     Http,
///     Grpc,
///     #[dynamic(rename = "ws")]
///     WebSocket,
/// }
///
/// // Using with attribute for newtype wrappers:
/// #[derive(Clone, Debug)]
/// pub struct Claims(serde_json::Value);
///
/// // Helper function to extract the inner value
/// fn extract_claims<'a>(c: &'a &'a Claims) -> &'a serde_json::Value {
///     &c.0
/// }
///
/// #[derive(DynamicType)]
/// pub struct HttpRequestRef<'a> {
///     method: &'a str,
///     #[dynamic(with = "extract_claims")]
///     claims: &'a Claims,
/// }
///
/// // Inside the cel crate itself:
/// #[derive(DynamicType)]
/// #[dynamic(crate = "crate")]
/// pub struct InternalType {
///     field: String,
/// }
/// ```
#[proc_macro_derive(DynamicType, attributes(dynamic))]
pub fn derive_dynamic_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Parse struct-level attributes
    let crate_path_str = get_struct_crate_path(&input.attrs);

    // Generate crate path - use custom path if specified, otherwise default to ::cel
    let crate_path: TokenStream2 = if let Some(path) = crate_path_str {
        path.parse().unwrap()
    } else {
        quote! { ::cel }
    };

    // Dispatch based on data type
    match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(FieldsNamed { named, .. }) => {
                // Named struct - existing behavior
                derive_for_named_struct(&input, named, &crate_path)
            }
            Fields::Unnamed(fields) => {
                // Tuple struct - check for single field (newtype pattern)
                if fields.unnamed.len() == 1 {
                    derive_for_newtype_struct(&input, &crate_path)
                } else {
                    syn::Error::new_spanned(
                        name,
                        "DynamicType can only be derived for tuple structs with a single field",
                    )
                    .to_compile_error()
                    .into()
                }
            }
            Fields::Unit => {
                syn::Error::new_spanned(
                    name,
                    "DynamicType can only be derived for structs with named fields or single-field tuple structs",
                )
                .to_compile_error()
                .into()
            }
        },
        Data::Enum(e) => {
            // Enum - check that all variants are unit variants
            if e.variants.iter().all(|v| matches!(v.fields, Fields::Unit)) {
                derive_for_unit_enum(&input, &e.variants, &crate_path)
            } else {
                syn::Error::new_spanned(
                    name,
                    "DynamicType can only be derived for unit enums (all variants must have no data)",
                )
                .to_compile_error()
                .into()
            }
        }
        Data::Union(_) => {
            syn::Error::new_spanned(name, "DynamicType cannot be derived for unions")
                .to_compile_error()
                .into()
        }
    }
}

/// Derive DynamicType for a newtype tuple struct (single field)
fn derive_for_newtype_struct(input: &DeriveInput, crate_path: &TokenStream2) -> TokenStream {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let generated = quote! {
        impl #impl_generics #crate_path::types::dynamic::DynamicType for #name #ty_generics #where_clause {
            fn auto_materialize(&self) -> bool {
                self.0.auto_materialize()
            }

            fn materialize(&self) -> #crate_path::Value<'_> {
                self.0.materialize()
            }

            fn field(&self, field: &str) -> ::core::option::Option<#crate_path::Value<'_>> {
                self.0.field(field)
            }
        }

        impl #impl_generics #crate_path::types::dynamic::DynamicFlatten for #name #ty_generics #where_clause {
            fn materialize_into<'__cel_a>(&'__cel_a self, __cel_map: &mut ::vector_map::VecMap<#crate_path::objects::KeyRef<'__cel_a>, #crate_path::Value<'__cel_a>>) {
                #crate_path::types::dynamic::DynamicFlatten::materialize_into(&self.0, __cel_map);
            }
        }
    };

    generated.into()
}

/// Derive DynamicType for a unit enum (all variants have no data)
fn derive_for_unit_enum(
    input: &DeriveInput,
    variants: &syn::punctuated::Punctuated<Variant, syn::token::Comma>,
    crate_path: &TokenStream2,
) -> TokenStream {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Generate match arms for materialize
    let materialize_arms: TokenStream2 = variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;
            let variant_name =
                get_variant_rename(&variant.attrs).unwrap_or_else(|| variant_ident.to_string());
            quote! {
                Self::#variant_ident => #crate_path::Value::String(#variant_name.into()),
            }
        })
        .collect();

    let generated = quote! {
        impl #impl_generics #crate_path::types::dynamic::DynamicType for #name #ty_generics #where_clause {
            fn auto_materialize(&self) -> bool {
                true
            }

            fn materialize(&self) -> #crate_path::Value<'_> {
                match self {
                    #materialize_arms
                }
            }
        }
    };

    generated.into()
}

/// Derive DynamicType for a named struct (original implementation)
fn derive_for_named_struct(
    input: &DeriveInput,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    crate_path: &TokenStream2,
) -> TokenStream {
    let name = &input.ident;

    // Get struct-level rename_all setting
    let rename_all = get_rename_all(&input.attrs);

    // Filter and process fields
    let processed_fields: Result<Vec<_>, syn::Error> = fields
        .iter()
        .filter(|f| !has_field_attr_combined(&f.attrs, "skip"))
        .map(|f| {
            let ident = f.ident.as_ref().unwrap();
            // Apply rename logic: explicit rename > rename_all > original name
            let name = get_field_rename_combined(&f.attrs)
                .unwrap_or_else(|| apply_rename_all(&ident.to_string(), rename_all.as_deref()));
            let ty = &f.ty;
            // Check if the type is a reference type
            let is_ref = matches!(ty, syn::Type::Reference(_));
            let with_expr = get_field_with_expr(&f.attrs);
            let with_value_expr = get_field_with_value_expr(&f.attrs);
            let is_flatten = has_field_attr(&f.attrs, "flatten");
            let skip_serializing_if = get_field_skip_serializing_if(&f.attrs);

            // Check for conflicting attributes
            if with_expr.is_some() && with_value_expr.is_some() {
                return Err(syn::Error::new_spanned(
                    f,
                    "Cannot use both `with` and `with_value` attributes on the same field",
                ));
            }

            // flatten conflicts with all other attributes except skip (already filtered)
            if is_flatten
                && (with_expr.is_some()
                    || with_value_expr.is_some()
                    || get_field_rename_combined(&f.attrs).is_some()
                    || skip_serializing_if.is_some())
            {
                return Err(syn::Error::new_spanned(
                    f,
                    "Cannot use `flatten` with `with`, `with_value`, `rename`, or `skip_serializing_if` attributes",
                ));
            }

            Ok((
                ident,
                name,
                ty,
                is_ref,
                with_expr,
                with_value_expr,
                is_flatten,
                skip_serializing_if,
            ))
        })
        .collect();

    let processed_fields = match processed_fields {
        Ok(fields) => fields,
        Err(e) => return e.to_compile_error().into(),
    };

    // Separate normal fields from flattened fields
    let (normal_fields, flatten_fields): (Vec<_>, Vec<_>) = processed_fields.iter().partition(
        |(
            _ident,
            _name,
            _ty,
            _is_ref,
            _with_expr,
            _with_value_expr,
            is_flatten,
            _skip_serializing_if,
        )| !is_flatten,
    );

    let field_count = normal_fields.len();

    // Generate materialize body
    let materialize_inserts: TokenStream2 = normal_fields
        .iter()
        .map(|(ident, name, _ty, _is_ref, with_expr, with_value_expr, _is_flatten, skip_serializing_if)| {
            let insert_code = if let Some(expr_str) = with_value_expr {
                // Parse the helper function path for with_value
                let parsed_expr: syn::Expr = match syn::parse_str(expr_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("Failed to parse `with_value` expression `{}`: {}", expr_str, e)
                        )
                        .to_compile_error();
                    }
                };
                // Convert the parsed expression to tokens
                let expr_tokens = parsed_expr.to_token_stream();
                // Call the helper and use returned Value directly (no maybe_materialize)
                quote! {
                    __cel_map.insert(
                        #crate_path::objects::KeyRef::from(#name),
                        (#expr_tokens)(&self.#ident),
                    );
                }
            } else if let Some(expr_str) = with_expr {
                // Parse the closure expression as a proper Expr for better diagnostics
                let parsed_expr: syn::Expr = match syn::parse_str(expr_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("Failed to parse `with` expression `{}`: {}", expr_str, e)
                        )
                        .to_compile_error();
                    }
                };
                // Convert the parsed expression to tokens
                let expr_tokens = parsed_expr.to_token_stream();
                // Call the closure and let maybe_materialize handle the result
                quote! {
                    __cel_map.insert(
                        #crate_path::objects::KeyRef::from(#name),
                        #crate_path::types::dynamic::maybe_materialize((#expr_tokens)(&self.#ident)),
                    );
                }
            } else {
                // Always pass a reference to maybe_materialize
                quote! {
                    __cel_map.insert(
                        #crate_path::objects::KeyRef::from(#name),
                        #crate_path::types::dynamic::maybe_materialize(&self.#ident),
                    );
                }
            };

            // Wrap with skip_serializing_if check if present
            if let Some(skip_fn_str) = skip_serializing_if {
                let skip_fn: syn::Expr = match syn::parse_str(skip_fn_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("Failed to parse `skip_serializing_if` expression `{}`: {}", skip_fn_str, e)
                        )
                        .to_compile_error();
                    }
                };
                let skip_fn_tokens = skip_fn.to_token_stream();
                quote! {
                    if !(#skip_fn_tokens)(&self.#ident) {
                        #insert_code
                    }
                }
            } else {
                insert_code
            }
        })
        .collect();

    // Generate flatten field merging code
    let flatten_merges: TokenStream2 = flatten_fields
        .iter()
        .map(
            |(
                ident,
                _name,
                _ty,
                _is_ref,
                _with_expr,
                _with_value_expr,
                _is_flatten,
                _skip_serializing_if,
            )| {
                quote! {
                    // Materialize the flattened field directly into the map
                    #crate_path::types::dynamic::DynamicFlatten::materialize_into(&self.#ident, __cel_map);
                }
            },
        )
        .collect();

    // Generate field match arms
    let field_arms: TokenStream2 = normal_fields
        .iter()
        .map(|(ident, name, ty, _is_ref, with_expr, with_value_expr, _is_flatten, skip_serializing_if)| {
            let field_value = if let Some(expr_str) = with_value_expr {
                // Parse the helper function path for with_value
                let parsed_expr: syn::Expr = match syn::parse_str(expr_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!(
                                "Failed to parse `with_value` expression `{}`: {}",
                                expr_str, e
                            ),
                        )
                        .to_compile_error();
                    }
                };
                // Convert the parsed expression to tokens
                let expr_tokens = parsed_expr.to_token_stream();
                // Call the helper and use returned Value directly (no maybe_materialize)
                quote! {
                    (#expr_tokens)(&self.#ident)
                }
            } else if let Some(expr_str) = with_expr {
                // Parse the closure expression as a proper Expr for better diagnostics
                let parsed_expr: syn::Expr = match syn::parse_str(expr_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("Failed to parse `with` expression `{}`: {}", expr_str, e),
                        )
                        .to_compile_error();
                    }
                };
                // Convert the parsed expression to tokens
                let expr_tokens = parsed_expr.to_token_stream();
                // Generate code with explicit type annotation for better type inference
                quote! {
                    {
                        let __field_ref: &#ty = &self.#ident;
                        #crate_path::types::dynamic::maybe_materialize((#expr_tokens)(__field_ref))
                    }
                }
            } else {
                // Always pass a reference to maybe_materialize
                quote! {
                    #crate_path::types::dynamic::maybe_materialize(&self.#ident)
                }
            };

            // Wrap with skip_serializing_if check if present
            if let Some(skip_fn_str) = skip_serializing_if {
                let skip_fn: syn::Expr = match syn::parse_str(skip_fn_str) {
                    Ok(expr) => expr,
                    Err(e) => {
                        return syn::Error::new(
                            proc_macro2::Span::call_site(),
                            format!("Failed to parse `skip_serializing_if` expression `{}`: {}", skip_fn_str, e)
                        )
                        .to_compile_error();
                    }
                };
                let skip_fn_tokens = skip_fn.to_token_stream();
                quote! {
                    #name => {
                        if (#skip_fn_tokens)(&self.#ident) {
                            ::core::option::Option::None
                        } else {
                            ::core::option::Option::Some(#field_value)
                        }
                    },
                }
            } else {
                quote! {
                    #name => ::core::option::Option::Some(#field_value),
                }
            }
        })
        .collect();

    // Generate fallback to flattened fields
    let flatten_fallback: TokenStream2 = if !flatten_fields.is_empty() {
        let flatten_checks = flatten_fields.iter().map(|(ident, _name, _ty, _is_ref, _with_expr, _with_value_expr, _is_flatten, _skip_serializing_if)| {
            quote! {
                if let ::core::option::Option::Some(val) = #crate_path::types::dynamic::DynamicType::field(&self.#ident, field) {
                    return ::core::option::Option::Some(val);
                }
            }
        });
        quote! {
            #(#flatten_checks)*
        }
    } else {
        quote! {}
    };

    // Handle generics - we need to support both lifetimes and type parameters
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let generated = quote! {
        impl #impl_generics #crate_path::types::dynamic::DynamicType for #name #ty_generics #where_clause {
            fn materialize(&self) -> #crate_path::Value<'_> {
                let mut m = ::vector_map::VecMap::with_capacity(#field_count);
                #crate_path::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
                #crate_path::Value::Map(#crate_path::objects::MapValue::Borrow(m))
            }

            fn field(&self, field: &str) -> ::core::option::Option<#crate_path::Value<'_>> {
                match field {
                    #field_arms
                    _ => {
                        #flatten_fallback
                        ::core::option::Option::None
                    }
                }
            }
        }

        impl #impl_generics #crate_path::types::dynamic::DynamicFlatten for #name #ty_generics #where_clause {
            fn materialize_into<'__cel_a>(&'__cel_a self, __cel_map: &mut ::vector_map::VecMap<#crate_path::objects::KeyRef<'__cel_a>, #crate_path::Value<'__cel_a>>) {
                #materialize_inserts
                #flatten_merges
            }
        }
    };

    generated.into()
}

/// Check if a field has a specific attribute
fn has_field_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::Path(path)) = attr.parse_args::<Meta>() {
                return path.is_ident(name);
            }
        }
        false
    })
}

/// Get the rename value for a field
fn get_field_rename(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("rename") {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

/// Get the crate path from struct-level attributes
fn get_struct_crate_path(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("crate") {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

/// Get the `with` expression for a field (closure to transform the value)
fn get_field_with_expr(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("with") {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

/// Get the `with_value` expression for a field (function that returns Value directly)
fn get_field_with_value_expr(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("with_value") {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

/// Get the `skip_serializing_if` expression for a field (checks both dynamic and serde)
/// Prefers dynamic over serde if both are present
fn get_field_skip_serializing_if(attrs: &[Attribute]) -> Option<String> {
    // First check dynamic
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("skip_serializing_if") {
                    return Some(lit_str.value());
                }
            }
        }
    }

    // Fall back to serde - need to parse as a list to handle multiple arguments
    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::token::Comma>::parse_terminated,
            ) {
                for meta in list {
                    if let Meta::NameValue(MetaNameValue {
                        path,
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: Lit::Str(lit_str),
                                ..
                            }),
                        ..
                    }) = meta
                    {
                        if path.is_ident("skip_serializing_if") {
                            return Some(lit_str.value());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Get the rename value for an enum variant
fn get_variant_rename(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("rename") {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

/// Get the rename_all value from struct-level attributes
/// Checks both #[dynamic(rename_all = "...")] and #[serde(rename_all = "...")]
/// Prefers dynamic over serde if both are present
fn get_rename_all(attrs: &[Attribute]) -> Option<String> {
    // First check for dynamic attribute
    for attr in attrs {
        if attr.path().is_ident("dynamic") {
            if let Ok(Meta::NameValue(MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: Lit::Str(lit_str),
                        ..
                    }),
                ..
            })) = attr.parse_args::<Meta>()
            {
                if path.is_ident("rename_all") {
                    return Some(lit_str.value());
                }
            }
        }
    }

    // Fall back to serde attribute - need to parse as a list to handle multiple arguments
    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::token::Comma>::parse_terminated,
            ) {
                for meta in list {
                    if let Meta::NameValue(MetaNameValue {
                        path,
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: Lit::Str(lit_str),
                                ..
                            }),
                        ..
                    }) = meta
                    {
                        if path.is_ident("rename_all") {
                            return Some(lit_str.value());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Apply rename_all transformation to a field name
fn apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("camelCase") => to_camel_case(name),
        Some("lowercase") => name.to_lowercase(),
        _ => name.to_string(),
    }
}

/// Convert snake_case to camelCase
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for (i, ch) in s.chars().enumerate() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            // First character should be lowercase
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }

    result
}

/// Check if a field has a specific attribute (checks both dynamic and serde)
/// Prefers dynamic over serde if both are present
fn has_field_attr_combined(attrs: &[Attribute], name: &str) -> bool {
    // First check dynamic
    if has_field_attr(attrs, name) {
        return true;
    }

    // Fall back to serde - need to parse as a list to handle multiple arguments
    attrs.iter().any(|attr| {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::token::Comma>::parse_terminated,
            ) {
                for meta in list {
                    if let Meta::Path(path) = meta {
                        if path.is_ident(name) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    })
}

/// Get the rename value for a field (checks both dynamic and serde)
/// Prefers dynamic over serde if both are present
fn get_field_rename_combined(attrs: &[Attribute]) -> Option<String> {
    // First check dynamic
    if let Some(name) = get_field_rename(attrs) {
        return Some(name);
    }

    // Fall back to serde - need to parse as a list to handle multiple arguments
    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::token::Comma>::parse_terminated,
            ) {
                for meta in list {
                    if let Meta::NameValue(MetaNameValue {
                        path,
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: Lit::Str(lit_str),
                                ..
                            }),
                        ..
                    }) = meta
                    {
                        if path.is_ident("rename") {
                            return Some(lit_str.value());
                        }
                    }
                }
            }
        }
    }

    None
}
