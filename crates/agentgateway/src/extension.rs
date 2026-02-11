//! Extension points for agentgateway
//!
//! This module provides extension mechanisms that allow downstream code
//! to inject custom functionality without modifying the core library.

#[cfg(feature = "schema")]
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use parking_lot::RwLock;
use prost::Message;
use serde::{Deserialize, Serialize};

/// A dynamic protobuf message that can be serialized/deserialized as JSON.
///
/// This trait makes `prost::Message` work with serde for JSON serialization
/// while preserving the ability to work with protobuf bytes for XDS.
pub trait DynamicProtoMessage: Send + Sync + std::fmt::Debug {
	/// Get the type URL for this message (e.g., "type.googleapis.com/mycompany.auth.Config")
	fn type_url(&self) -> &str;

	/// Serialize to JSON
	fn to_json(&self) -> Result<serde_json::Value, anyhow::Error>;

	/// Clone into a new box
	fn clone_box(&self) -> Box<dyn DynamicProtoMessage>;
	/// Get a reference to the inner message as `Any` for downcasting
	fn as_any(&self) -> &dyn std::any::Any;
}

pub trait HasRegistry<T> {
	fn registry() -> &'static Registry<T>;
}

/// Configuration as a dynamic protobuf message (can serialize to JSON)
pub struct Extension<T> {
	pub name: String,
	pub config: Box<dyn crate::extension::DynamicProtoMessage>,
	/// The resolved handler instance (eagerly resolved during deserialization)
	pub handler: T,
}

impl<T> Extension<T> {
	pub fn new(name: &str, config: Box<dyn DynamicProtoMessage>, handler: T) -> Self {
		Self {
			name: name.to_string(),
			config,
			handler,
		}
	}
}

impl<T: 'static> Extension<T>
where
	Extension<T>: HasRegistry<T>,
{
	/// Resolve a custom auth handler from protobuf Any (for XDS)
	pub fn resolve_extension(
		ext: &crate::types::proto::agent::Extension,
	) -> Result<Self, anyhow::Error> {
		let config = ext
			.config
			.as_ref()
			.ok_or_else(|| anyhow::anyhow!("Extension config is required"))?;
		Self::resolve_any(ext.name.as_str(), config)
	}
	pub fn resolve_any(name: &str, any: &prost_types::Any) -> Result<Self, anyhow::Error> {
		if name.is_empty() {
			anyhow::bail!("Extension name is required");
		}
		let type_url = any.type_url.as_str();
		let registry = <Self as HasRegistry<T>>::registry();

		let reg = registry.get(type_url).ok_or_else(|| {
                    anyhow::anyhow!(
                        concat!("Unknown Extension for type '{}'. Make sure to register the handler before loading configuration."),
                        type_url
                    )
                })?;

		let (u, c) = reg.resolve_any(name, any)?;
		Ok(Self::new(name, c, u))
	}

	pub fn register<C>(factory: impl Fn(&str, &C) -> Result<T, anyhow::Error> + Send + Sync + 'static)
	where
		C: prost::Message
			+ Default
			+ Clone
			+ Send
			+ Sync
			+ std::fmt::Debug
			+ serde::Serialize
			+ for<'de> serde::Deserialize<'de>
			+ prost::Name
			+ 'static,
	{
		let registry = <Self as HasRegistry<T>>::registry();
		registry.register::<C>(factory);
	}
}

impl<T> std::fmt::Debug for Extension<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Extension")
			.field("config", &self.config)
			.finish()
	}
}

#[cfg(feature = "schema")]
impl<T> schemars::JsonSchema for Extension<T> {
	fn schema_name() -> Cow<'static, str> {
		"Extension".into()
	}

	fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
				"type": ["object"],
				"properties": {
					"name": {
						"type": "string"
					},
					"config": {
						"type": "object"
					}
				},
				"required": ["name", "config"]
		})
	}
}

// Manual Deserialize implementation to handle both standard and custom auth types
impl<'de, T: 'static> serde::Deserialize<'de> for Extension<T>
where
	Extension<T>: HasRegistry<T>,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		use serde::de::Error;

		// First deserialize into a generic Value to inspect it
		let value = serde_json::Value::deserialize(deserializer)?;
		// get name and config
		let name = value
			.get("name")
			.and_then(serde_json::Value::as_str)
			.map(|s| s.trim())
			.filter(|s| !s.is_empty())
			.ok_or_else(|| Error::custom("`name` field is required and must be a non-empty string"))?
			.to_string();
		let config = value
			.get("config")
			.ok_or_else(|| Error::custom("`config` field is required"))?
			.clone();

		// Check if this is a custom auth with @type field (google.protobuf.Any format)
		if let Some(obj) = config.as_object()
			&& let Some(type_url) = obj.get("@type").and_then(serde_json::Value::as_str)
		{
			// This is a custom auth handler
			// Remove the @type field and keep the rest as config
			let mut config_obj = obj.clone();
			config_obj.remove("@type");
			let config_json = serde_json::Value::Object(config_obj);

			// Use extension module to resolve handler and config
			let registry = <Self as HasRegistry<T>>::registry();
			let reg = registry.get(type_url).ok_or_else(|| {
				Error::custom(format!(
					"Failed to resolve custom auth handler for : {}",
					type_url
				))
			})?;

			let (handler, config) = reg
				.resolve(name.as_str(), &config_json)
				.map_err(|e| Error::custom(format!("Failed to resolve custom auth handler: {}", e)))?;

			return Ok(Extension {
				name,
				config,
				handler,
			});
		}

		Err(Error::custom(format!(
			"Invalid {} configuration. missing @type field.",
			std::any::type_name::<T>()
		)))
	}
}

impl<T> serde::Serialize for Extension<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		use serde::ser::SerializeMap;

		// Use DynamicProtoMessage::to_json() to serialize the config
		let mut json = self.config.to_json().map_err(serde::ser::Error::custom)?;
		let mut obj = serde_json::Map::new();
		obj.insert(
			"name".to_string(),
			serde_json::Value::String(self.name.clone()),
		);

		if let Some(o) = json.as_object_mut() {
			o.insert(
				"@type".to_string(),
				serde_json::Value::String(self.config.type_url().into()),
			);
		}

		obj.insert("config".to_string(), json);

		let mut map = serializer.serialize_map(None)?;
		// Merge in the config fields
		for (key, value) in obj {
			map.serialize_entry(&key, &value)?;
		}

		map.end()
	}
}

impl Clone for Box<dyn DynamicProtoMessage> {
	fn clone(&self) -> Self {
		self.clone_box()
	}
}

/// Concrete implementation of DynamicProtoMessage for any prost Message
struct ProtoMessageWrapper<T: Message + Clone + Send + Sync + std::fmt::Debug + 'static> {
	type_url: String,
	message: T,
}

impl<T> std::fmt::Debug for ProtoMessageWrapper<T>
where
	T: Message + Clone + Send + Sync + std::fmt::Debug + 'static,
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ProtoMessageWrapper")
			.field("type_url", &self.type_url)
			.field("message", &self.message)
			.finish()
	}
}

impl<T> DynamicProtoMessage for ProtoMessageWrapper<T>
where
	T: Message + Clone + Send + Sync + std::fmt::Debug + Serialize + 'static,
{
	fn type_url(&self) -> &str {
		&self.type_url
	}

	fn to_json(&self) -> Result<serde_json::Value, anyhow::Error> {
		serde_json::to_value(&self.message).context("Failed to serialize message to JSON")
	}

	fn clone_box(&self) -> Box<dyn DynamicProtoMessage> {
		Box::new(ProtoMessageWrapper {
			type_url: self.type_url.clone(),
			message: self.message.clone(),
		})
	}

	fn as_any(&self) -> &dyn std::any::Any {
		&self.message
	}
}

// Factory type for creating handlers from config
type Factory<T> = Arc<dyn Fn(&str, &dyn std::any::Any) -> Result<T, anyhow::Error> + Send + Sync>;

type FromJsonFunc =
	dyn Fn(&serde_json::Value) -> Result<Box<dyn DynamicProtoMessage>, anyhow::Error> + Send + Sync;
type FromSliceFunc =
	dyn Fn(&[u8]) -> Result<Box<dyn DynamicProtoMessage>, anyhow::Error> + Send + Sync;
struct Registration<T> {
	factory: Factory<T>,
	from_json: Box<FromJsonFunc>,
	from_slice: Box<FromSliceFunc>,
	type_url: String,
}

impl<T> Registration<T> {
	fn new<C>(factory: impl Fn(&str, &C) -> Result<T, anyhow::Error> + Send + Sync + 'static) -> Self
	where
		C: Message
			+ Default
			+ Clone
			+ Send
			+ Sync
			+ std::fmt::Debug
			+ Serialize
			+ for<'de> Deserialize<'de>
			+ prost::Name
			+ 'static,
	{
		// Construct the type URL``
		let type_url = format!("type.googleapis.com/{}.{}", C::PACKAGE, C::NAME);

		// Wrap the factory to work with DynamicProtoMessage

		let type_url_for_json = type_url.clone();
		let type_url_for_slice = type_url.clone();
		Registration {
			type_url,
			factory: Arc::new(move |name: &str, config: &dyn std::any::Any| {
				// Deserialize from JSON (config is already a DynamicProtoMessage)
				if let Some(cfg) = config.downcast_ref::<C>() {
					return factory(name, cfg);
				}
				anyhow::bail!("Failed to downcast config to expected type {}", C::NAME);
			}),
			from_json: Box::new(move |json| {
				Ok(Box::new(ProtoMessageWrapper {
					type_url: type_url_for_json.clone(),
					message: serde_json::from_value::<C>(json.clone())?,
				}))
			}),
			from_slice: Box::new(move |bytes| {
				Ok(Box::new(ProtoMessageWrapper {
					type_url: type_url_for_slice.clone(),
					message: C::decode(bytes)?,
				}))
			}),
		}
	}

	pub(crate) fn resolve(
		&self,
		name: &str,
		config_json: &serde_json::Value,
	) -> Result<(T, Box<dyn DynamicProtoMessage>), anyhow::Error> {
		self.resolve_dynamic(name, (self.from_json)(config_json)?)
	}

	pub(crate) fn resolve_any(
		&self,
		name: &str,
		any: &prost_types::Any,
	) -> Result<(T, Box<dyn DynamicProtoMessage>), anyhow::Error> {
		self.resolve_dynamic(name, (self.from_slice)(&any.value)?)
	}

	fn resolve_dynamic(
		&self,
		name: &str,
		config: Box<dyn DynamicProtoMessage>,
	) -> Result<(T, Box<dyn DynamicProtoMessage>), anyhow::Error> {
		// Create handler
		let handler = (self.factory)(name, config.as_any())?;
		Ok((handler, config))
	}
}

pub struct Registry<T> {
	registrations: RwLock<HashMap<String, Arc<Registration<T>>>>,
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Self {
			registrations: RwLock::new(HashMap::new()),
		}
	}
}

impl<T> Registry<T> {
	fn insert(&self, type_url: String, registration: Registration<T>) {
		self
			.registrations
			.write()
			.insert(type_url, Arc::new(registration));
	}

	fn get(&self, type_url: &str) -> Option<Arc<Registration<T>>> {
		self.registrations.read().get(type_url).cloned()
	}

	fn register<C>(
		&self,
		factory: impl Fn(&str, &C) -> Result<T, anyhow::Error> + Send + Sync + 'static,
	) where
		C: Message
			+ Default
			+ Clone
			+ Send
			+ Sync
			+ std::fmt::Debug
			+ Serialize
			+ for<'de> Deserialize<'de>
			+ prost::Name
			+ 'static,
	{
		let reg = Registration::new::<C>(factory);
		// Register with full type URL
		self.insert(reg.type_url.clone(), reg);
	}
}

/// Macro to generate extension boilerplate for a given handler trait.
///
/// This generates an Extension alias for the Extension<T>, a static registry,
/// and associates the registry with the trait.
///
/// # Example
///
/// ```ignore
/// define_extension_point!(
///     BackendAuth, // Name of the extension point
///     BackendAuthHandler, // Trait for the handler
/// );
/// ```
#[macro_export]
macro_rules! define_extension_point {
    (
        $name:ident,
        $trait:ty
    ) => {
        paste::paste! {
            static [<$name:snake:upper _REGISTRY>]: once_cell::sync::Lazy<$crate::extension::Registry<Box<dyn $trait>>> = once_cell::sync::Lazy::new(Default::default);
            pub type [<$name Extension>] = $crate::extension::Extension<Box<dyn $trait>>;

            impl $crate::extension::HasRegistry<Box<dyn $trait>> for [<$name Extension>] {
                fn registry() -> &'static $crate::extension::Registry<Box<dyn $trait>> {
                    &*[<$name:snake:upper _REGISTRY>]
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	macro_rules! test_fixture {
		() => {
			// Define a test trait for extension testing
			pub trait TestHandler: Send + Sync + std::fmt::Debug {
				fn handle(&self) -> String;
			}

			// Define a dummy handler implementation
			#[derive(Debug)]
			#[allow(dead_code)] // In some tests, this is not used, as they test failures
			struct DummyTestHandler {
				config_value: String,
			}

			impl TestHandler for DummyTestHandler {
				fn handle(&self) -> String {
					format!("Handled: {}", self.config_value)
				}
			}

			// Create an extension point for testing
			define_extension_point!(Test, TestHandler);
		};
	}
	#[test]
	fn test_extension_json_round_trip() {
		test_fixture!();
		// Register handler for TestExtension
		TestExtension::register(|name, config: &crate::types::proto::agent::TestExtension| {
			assert_eq!(name, "my-extension");
			Ok(Box::new(DummyTestHandler {
				config_value: config.value.clone(),
			}) as Box<dyn TestHandler>)
		});

		// Create a TestExtension with a name field
		let test_config = crate::types::proto::agent::TestExtension {
			value: "test-handler".to_string(),
		};

		let any = prost_types::Any::from_msg(&test_config).expect("Failed to create Any");
		let proto_ext = crate::types::proto::agent::Extension {
			name: "my-extension".to_string(),
			config: Some(any),
		};
		let extension =
			TestExtension::resolve_extension(&proto_ext).expect("Failed to resolve extension");

		// Serialize to JSON
		let json = serde_json::to_value(&extension).expect("Failed to serialize");

		// Verify structure
		let expected = json!({
			"name": "my-extension",
			"config": {
				"@type": "type.googleapis.com/agentgateway.dev.test.TestExtension",
				"value": "test-handler"
			}
		});
		// Check for @type field
		assert_eq!(json, expected);

		// Deserialize back
		let deserialized: TestExtension = serde_json::from_value(json).expect("Failed to deserialize");

		// Verify handler works
		assert_eq!(deserialized.name, "my-extension");
		assert_eq!(deserialized.handler.handle(), "Handled: test-handler");

		// Verify config is preserved
		let config_any = deserialized.config.as_any();
		let config_typed = config_any
			.downcast_ref::<crate::types::proto::agent::TestExtension>()
			.expect("Config should downcast to TestExtension");
		assert_eq!(config_typed.value, "test-handler");
	}

	#[test]
	fn test_extension_protobuf_any_deserialization() {
		test_fixture!();
		// Register handler for TestExtension
		TestExtension::register(|name, config: &crate::types::proto::agent::TestExtension| {
			assert_eq!(name, "proto-extension");
			Ok(Box::new(DummyTestHandler {
				config_value: config.value.clone(),
			}) as Box<dyn TestHandler>)
		});

		// Create a TestExtension protobuf message
		let test_config = crate::types::proto::agent::TestExtension {
			value: "protobuf-test".to_string(),
		};

		// Create protobuf Any
		let any = prost_types::Any::from_msg(&test_config).expect("Failed to create Any");

		// Deserialize using resolve_any
		let extension = TestExtension::resolve_any("proto-extension", &any)
			.expect("Failed to resolve extension from Any");

		// Verify extension
		assert_eq!(extension.name, "proto-extension");
		assert_eq!(extension.handler.handle(), "Handled: protobuf-test");

		// Verify config is preserved
		let config_any = extension.config.as_any();
		let config_typed = config_any
			.downcast_ref::<crate::types::proto::agent::TestExtension>()
			.expect("Config should downcast to TestExtension");
		assert_eq!(config_typed.value, "protobuf-test");
	}

	#[test]
	fn test_extension_protobuf_any_via_proto_extension() {
		test_fixture!();
		// Register handler for TestExtension
		TestExtension::register(|name, config: &crate::types::proto::agent::TestExtension| {
			assert_eq!(name, "xds-extension");
			Ok(Box::new(DummyTestHandler {
				config_value: config.value.clone(),
			}) as Box<dyn TestHandler>)
		});

		// Create a TestExtension protobuf message
		let test_config = crate::types::proto::agent::TestExtension {
			value: "xds-test".to_string(),
		};

		// Create the proto Extension wrapper
		let proto_ext = crate::types::proto::agent::Extension {
			name: "xds-extension".to_string(),
			config: Some(prost_types::Any::from_msg(&test_config).expect("Failed to create Any")),
		};

		// Resolve using resolve_extension (simulates XDS path)
		let extension =
			TestExtension::resolve_extension(&proto_ext).expect("Failed to resolve extension");

		// Verify extension
		assert_eq!(extension.name, "xds-extension");
		assert_eq!(extension.handler.handle(), "Handled: xds-test");

		// Verify config is preserved
		let config_any = extension.config.as_any();
		let config_typed = config_any
			.downcast_ref::<crate::types::proto::agent::TestExtension>()
			.expect("Config should downcast to TestExtension");
		assert_eq!(config_typed.value, "xds-test");
	}

	#[test]
	fn test_extension_unregistered_type_error() {
		test_fixture!();
		TestExtension::register(
			|_name, _config: &crate::types::proto::agent::TestExtension| {
				panic!("Factory should not be called when type is not registered");
			},
		);
		// Don't register any handler
		// Create a fake Any with unregistered type URL
		let any = prost_types::Any {
			type_url: "type.googleapis.com/unknown.UnregisteredType".to_string(),
			value: vec![],
		};

		// Try to resolve - should fail with clear error
		let result = TestExtension::resolve_any("test", &any);

		assert!(result.is_err(), "Should fail for unregistered type");
		let err = result.unwrap_err().to_string();
		assert!(
			err.contains("Unknown Extension"),
			"Error should mention unknown extension, got: {}",
			err
		);
		assert!(
			err.contains("unknown.UnregisteredType"),
			"Error should mention the type URL, got: {}",
			err
		);
	}

	#[test]
	fn test_extension_missing_name_error() {
		test_fixture!();
		// Register handler
		TestExtension::register(
			|_name, _config: &crate::types::proto::agent::TestExtension| {
				panic!("Factory should not be called when name is missing");
			},
		);

		let test_config = crate::types::proto::agent::TestExtension {
			value: "test".to_string(),
		};

		let any = prost_types::Any::from_msg(&test_config).expect("Failed to create Any");

		// Try to resolve with empty name - should fail
		let result = TestExtension::resolve_any("", &any);

		assert!(result.is_err(), "Should fail for empty name");
		let err = result.unwrap_err().to_string();
		assert!(
			err.contains("Extension name is required"),
			"Error should mention missing name, got: {}",
			err
		);
	}

	#[test]
	fn test_extension_json_deserialization_missing_type() {
		test_fixture!();
		// Register handler
		TestExtension::register(
			|_name, _config: &crate::types::proto::agent::TestExtension| {
				panic!("Factory should not be called when @type is missing");
			},
		);

		// Create JSON without @type field
		let json = json!({
			"name": "test",
			"config": {
				"name": "test-config"
			}
		});

		// Try to deserialize - should fail
		let result = serde_json::from_value::<TestExtension>(json);

		assert!(result.is_err(), "Should fail when @type field is missing");
	}

	#[test]
	fn test_extension_multiple_types_no_cross_contamination() {
		test_fixture!();
		// Define a second test handler type
		trait SecondTestHandler: Send + Sync + std::fmt::Debug {
			fn process(&self) -> String;
		}

		#[derive(Debug)]
		struct SecondDummyHandler {
			value: String,
		}

		impl SecondTestHandler for SecondDummyHandler {
			fn process(&self) -> String {
				format!("Processed: {}", self.value)
			}
		}

		// Create second extension point
		define_extension_point!(SecondTest, SecondTestHandler);

		// Register handlers for first type
		TestExtension::register(
			|_name, config: &crate::types::proto::agent::TestExtension| {
				Ok(Box::new(DummyTestHandler {
					config_value: config.value.clone(),
				}) as Box<dyn TestHandler>)
			},
		);
		// Same extension point, differnt proto
		TestExtension::register(
			|_name, config: &crate::types::proto::agent::Test2Extension| {
				Ok(Box::new(DummyTestHandler {
					config_value: config.value2.clone(),
				}) as Box<dyn TestHandler>)
			},
		);

		// Different extension point, same proto as first
		SecondTestExtension::register(
			|_name, config: &crate::types::proto::agent::TestExtension| {
				Ok(Box::new(SecondDummyHandler {
					value: config.value.clone(),
				}) as Box<dyn SecondTestHandler>)
			},
		);

		// Create extensions
		let test_config = crate::types::proto::agent::TestExtension {
			value: "shared-config".to_string(),
		};
		let test2_config = crate::types::proto::agent::Test2Extension {
			value2: "shared2-config".to_string(),
		};

		let any = prost_types::Any::from_msg(&test_config).expect("Failed to create Any");
		let any2 = prost_types::Any::from_msg(&test2_config).expect("Failed to create Any");

		let first_ext =
			TestExtension::resolve_any("first", &any).expect("Failed to resolve first extension");

		let second_ext =
			SecondTestExtension::resolve_any("second", &any).expect("Failed to resolve second extension");

		let third_ext =
			TestExtension::resolve_any("third", &any2).expect("Failed to resolve third extension");

		// Verify each extension uses its own handler type correctly
		assert_eq!(first_ext.handler.handle(), "Handled: shared-config");
		assert_eq!(second_ext.handler.process(), "Processed: shared-config");
		assert_eq!(third_ext.handler.handle(), "Handled: shared2-config");

		// Verify they're independent - changes to one don't affect the other
		assert_eq!(first_ext.name, "first");
		assert_eq!(second_ext.name, "second");
		assert_eq!(third_ext.name, "third");
	}
}
