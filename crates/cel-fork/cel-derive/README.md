# cel-derive

Derive macros for the `cel` crate.

## Usage

Add `cel` to your dependencies (which will automatically include `cel-derive`):

```toml
[dependencies]
cel = "0.13"
```

Then use the `DynamicType` derive macro on your structs:

```rust
use cel::DynamicType;

#[derive(DynamicType)]
pub struct MyData<'a> {
    name: &'a str,
    value: i64,
    #[dynamic(skip)]
    internal_field: bool,
}
```

## Attributes

### Struct-level attributes

- `#[dynamic(crate = "path")]` - Specify the path to the `cel` crate. Useful when using this derive macro inside the `cel` crate itself or when the crate is re-exported under a different name.
  - Use `#[dynamic(crate = "crate")]` when deriving inside the cel crate itself
  - Use `#[dynamic(crate = "::cel")]` or omit for normal external usage
  - Use `#[dynamic(crate = "::my_crate::cel")]` if cel is re-exported from another crate

### Field-level attributes

- `#[dynamic(skip)]` - Skip this field in the generated implementation. The field will not be accessible from CEL expressions.
- `#[dynamic(rename = "new_name")]` - Use a different name for this field when accessed from CEL expressions.
- `#[dynamic(with = "function")]` - Transform the field value using a helper function before passing to `maybe_materialize`. The function receives a reference to the field (note: if the field is already a reference like `&'a T`, the function receives `&&'a T`) and should return a reference to a type that implements `DynamicType + DynamicValueVtable`. Useful for newtype wrappers or extracting inner values.

  **Important**: Due to type inference limitations, you must use a named helper function with explicit lifetime annotations rather than inline closures.

- `#[dynamic(with_value = "function")]` - Transform the field value using a helper function that returns a `Value` directly. The function receives a reference to the field and must return `Value<'_>`. This is useful for types that don't implement `DynamicType` but can be converted to CEL values (e.g., types implementing `AsRef<str>`).

  **Note**: Cannot be used together with `with` attribute on the same field.

## Example

```rust
use cel::DynamicType;

#[derive(DynamicType)]
pub struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    #[dynamic(rename = "statusCode")]
    status_code: i32,
    #[dynamic(skip)]
    internal_timestamp: u64,
}
```

### Using `with` attribute for newtype wrappers

```rust
use cel::DynamicType;

// Newtype wrapper around serde_json::Value
#[derive(Clone, Debug)]
pub struct Claims(pub serde_json::Value);

// Helper function to extract the inner value from Claims
// Note: the function receives &&Claims because claims field is &'a Claims
fn extract_claims<'a>(c: &'a &'a Claims) -> &'a serde_json::Value {
    &c.0
}

#[derive(DynamicType)]
pub struct HttpRequestRef<'a> {
    method: &'a str,
    path: &'a str,
    // Extract the inner serde_json::Value from the Claims newtype
    #[dynamic(with = "extract_claims")]
    claims: &'a Claims,
}
```

In this example, the `with` attribute uses a helper function to extract the inner `serde_json::Value` from the `Claims` newtype wrapper. The function receives `&&'a Claims` (a reference to the `&'a Claims` field) and returns `&serde_json::Value`. The explicit lifetime annotations are necessary for the compiler to properly infer the types.

### Using `with_value` attribute for custom types

The `with_value` attribute is useful when you have a field type that doesn't implement `DynamicType` but can be converted to a CEL value. For example, types from external crates like `http::Method`:

```rust
use cel::{DynamicType, Value};

// Helper function that converts http::Method to a CEL Value
// Note: the function receives &&http::Method because the field is &'a http::Method
fn method_to_value<'a>(method: &'a &'a http::Method) -> Value<'a> {
    Value::String(method.as_str().into())
}

#[derive(DynamicType)]
pub struct HttpRequest<'a> {
    #[dynamic(with_value = "method_to_value")]
    method: &'a http::Method,
    path: &'a str,
}
```

The key difference between `with` and `with_value`:
- `with` expects a function that returns a reference to something implementing `DynamicType`, which is then passed to `maybe_materialize`
- `with_value` expects a function that directly returns a `Value`, bypassing `maybe_materialize` entirely

Use `with_value` when:
- The type doesn't implement `DynamicType` and you can't or don't want to implement it
- You want direct control over the `Value` conversion
- You're working with types from external crates (like `http::Method`)

### Using inside the cel crate

When using `#[derive(DynamicType)]` inside the `cel` crate itself, you need to either:

1. Use the `crate` attribute:
```rust
#[derive(DynamicType)]
#[dynamic(crate = "crate")]
pub struct InternalType {
    field: String,
}
```

2. Or add an extern crate alias at the module level:
```rust
extern crate self as cel;

#[derive(DynamicType)]
pub struct InternalType {
    field: String,
}
```

## For Foreign Types

If you need to implement `DynamicType` for a type you don't own (like types from other crates), you can manually implement `DynamicType`:

```rust
use cel::types::dynamic::{DynamicType};

impl DynamicType for serde_json::Value {
    fn materialize(&self) -> cel::Value<'_> {
        cel::to_value(self).unwrap()
    }
    
    fn auto_materialize(&self) -> bool {
        false
    }
    
    fn field(&self, field: &str) -> Option<cel::Value<'_>> {
        // Custom field lookup logic
        None
    }
}
```
