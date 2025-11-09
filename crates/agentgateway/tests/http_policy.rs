use agentgateway::types::backend::HTTP;

#[test]
fn http_policy_is_http11() {
    // Default (unset): no version
    let pol = HTTP { version: None };
    assert!(!pol.is_http11());

    // HTTP/1.1
    let pol = HTTP { version: Some(http::Version::HTTP_11) };
    assert!(pol.is_http11());

    // HTTP/2
    let pol = HTTP { version: Some(http::Version::HTTP_2) };
    assert!(!pol.is_http11());
}

#[test]
  fn http_policy_yaml_formats() {
    // User-friendly format: "1.1"
    let yaml = r#"version: "1.1""#;
    let pol: HTTP = serde_yaml::from_str(yaml).expect("parse 1.1");
    assert_eq!(pol.version, Some(http::Version::HTTP_11));
    assert!(pol.is_http11());

    // Native format: "HTTP/1.1"
    let yaml = r#"version: "HTTP/1.1""#;
    let pol: HTTP = serde_yaml::from_str(yaml).expect("parse HTTP/1.1");
    assert_eq!(pol.version, Some(http::Version::HTTP_11));
    assert!(pol.is_http11());

    // User-friendly format: "2"
    let yaml = r#"version: "2""#;
    let pol: HTTP = serde_yaml::from_str(yaml).expect("parse 2");
    assert_eq!(pol.version, Some(http::Version::HTTP_2));
    assert!(!pol.is_http11());

    // Native format: "HTTP/2"
    let yaml = r#"version: "HTTP/2""#;
    let pol: HTTP = serde_yaml::from_str(yaml).expect("parse HTTP/2");
    assert_eq!(pol.version, Some(http::Version::HTTP_2));

    // Native format: "HTTP/2.0"
    let yaml = r#"version: "HTTP/2.0""#;
    let pol: HTTP = serde_yaml::from_str(yaml).expect("parse HTTP/2.0");
    assert_eq!(pol.version, Some(http::Version::HTTP_2));

      // Serialization produces user-friendly format
      let pol = HTTP { version: Some(http::Version::HTTP_11) };
      let yaml = serde_yaml::to_string(&pol).expect("serialize");
      assert!(yaml.contains("\"1.1\"") || yaml.contains("'1.1'"));

      let pol = HTTP { version: Some(http::Version::HTTP_2) };
      let yaml = serde_yaml::to_string(&pol).expect("serialize");
      assert!(yaml.contains("\"2\"") || yaml.contains("'2'"));
  }

  #[test]
  fn http_policy_yaml_invalid_values() {
      // Unknown version should fail to parse
      let yaml = r#"version: "3""#;
      let err = serde_yaml::from_str::<HTTP>(yaml).unwrap_err();
      assert!(err.to_string().contains("expected version"));

      let yaml = r#"version: "HTTP/3""#;
      let err = serde_yaml::from_str::<HTTP>(yaml).unwrap_err();
      assert!(err.to_string().contains("expected version"));
  }
