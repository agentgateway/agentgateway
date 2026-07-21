#![allow(dead_code)]

use once_cell::sync::Lazy;

use crate::llm::policy::pii::email_recognizer::EmailRecognizer;
use crate::llm::policy::pii::phone_recognizer::PhoneRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

mod api_key_recognizer;
mod aws_access_key_recognizer;
mod ca_sin_recognizer;
mod credit_card_recognizer;
mod email_recognizer;
mod gcp_api_key_recognizer;
mod github_token_recognizer;
mod jwt_recognizer;
mod pattern_recognizer;
mod phone_recognizer;
mod private_key_recognizer;
mod recognizer;
mod recognizer_result;
mod slack_token_recognizer;
mod url_recognizer;
mod us_ssn_recognizer;

pub static EMAIL: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(EmailRecognizer::new()));

pub static PHONE: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(PhoneRecognizer::new()));

pub static CC: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(credit_card_recognizer::CreditCardRecognizer::new()));

pub static SSN: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(us_ssn_recognizer::UsSsnRecognizer::new()));

pub static CA_SIN: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(ca_sin_recognizer::CaSinRecognizer::new()));

pub static API_KEY: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(api_key_recognizer::ApiKeyRecognizer::new()));

pub static PRIVATE_KEY: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(private_key_recognizer::PrivateKeyRecognizer::new()));

pub static GITHUB_TOKEN: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(github_token_recognizer::GithubTokenRecognizer::new()));

pub static AWS_ACCESS_KEY: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(aws_access_key_recognizer::AwsAccessKeyRecognizer::new()));

pub static SLACK_TOKEN: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(slack_token_recognizer::SlackTokenRecognizer::new()));

pub static JWT: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(jwt_recognizer::JwtRecognizer::new()));

pub static GCP_API_KEY: Lazy<Box<dyn Recognizer + Sync + Send + 'static>> =
	Lazy::new(|| Box::new(gcp_api_key_recognizer::GcpApiKeyRecognizer::new()));

#[allow(clippy::borrowed_box)]
pub fn recognizer(
	r: &Box<dyn Recognizer + Sync + Send + 'static>,
	text: &str,
) -> Vec<recognizer_result::RecognizerResult> {
	r.recognize(text)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
