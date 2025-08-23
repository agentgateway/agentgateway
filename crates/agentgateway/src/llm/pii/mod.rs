#![allow(dead_code)]

use once_cell::sync::Lazy;

use crate::llm::pii::email_recognizer::EmailRecognizer;
use crate::llm::pii::phone_recognizer::PhoneRecognizer;
use crate::llm::pii::recognizer::Recognizer;

mod credit_card_recognizer;
mod email_recognizer;
mod pattern_recognizer;
mod phone_recognizer;
mod recognizer;
mod recognizer_result;
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

#[allow(clippy::borrowed_box)]
pub fn recognizer(r: &Box<dyn Recognizer + Sync + Send + 'static>, text: &str) -> Vec<recognizer_result::RecognizerResult> {
	r.recognize(text)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
