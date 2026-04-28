//! Verify apache's htpasswd file
//!
//! Supports MD5, BCrypt, SHA1, Unix crypt

use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sha1::{Digest, Sha1};

use crate::md5::APR1_ID;

pub mod md5;

static BCRYPT_ID: &str = "$2y$";
static SHA1_ID: &str = "{SHA}";

pub struct Htpasswd<'a>(HashMap<Cow<'a, str>, Hash<'a>>);

#[derive(Debug, Eq, PartialEq)]
pub enum Hash<'a> {
	MD5(MD5Hash<'a>),
	BCrypt(Cow<'a, str>),
	SHA1(Cow<'a, str>),
	Crypt(Cow<'a, str>),
}

#[derive(Debug, Eq, PartialEq)]
pub struct MD5Hash<'a> {
	pub salt: Cow<'a, str>,
	pub hash: Cow<'a, str>,
}

impl Htpasswd<'static> {
	pub fn new(bytes: &str) -> Htpasswd<'static> {
		let lines = bytes.split('\n');
		let hashes = lines
			.filter_map(parse_hash_entry)
			.map(|(username, hash)| (Cow::Owned(username.to_string()), hash.to_owned()))
			.collect::<HashMap<_, _>>();
		Htpasswd(hashes)
	}
}

impl<'a> Htpasswd<'a> {
	pub fn check<S: AsRef<str>>(&self, username: S, password: S) -> bool {
		self
			.0
			.get(username.as_ref())
			.map(|hash| hash.check(password))
			.unwrap_or_default()
	}

	pub fn into_owned(self) -> Htpasswd<'static> {
		Htpasswd(
			self
				.0
				.into_iter()
				.map(|(username, hash)| (Cow::Owned(username.to_string()), hash.to_owned()))
				.collect(),
		)
	}
}

fn parse_hash_entry(entry: &'_ str) -> Option<(Cow<'_, str>, Hash<'_>)> {
	let separator = entry.find(':')?;
	let username = &entry[..separator];
	let hash_id = &entry[(separator + 1)..];
	Hash::parse(hash_id).map(|hash| (Cow::Borrowed(username), hash))
}

impl<'a> Hash<'a> {
	pub fn check<S: AsRef<str>>(&self, password: S) -> bool {
		let password = password.as_ref();
		match self {
			Hash::MD5(hash) => md5::md5_apr1_encode(password, &hash.salt).as_str() == hash.hash,
			Hash::BCrypt(hash) => bcrypt::verify(password, hash).unwrap_or(false),
			Hash::SHA1(hash) => BASE64_STANDARD.encode(Sha1::digest(password)).as_str() == *hash,
			Hash::Crypt(hash) => pwhash::unix_crypt::verify(password, hash),
		}
	}

	/// Parses the hash part of the htpasswd entry.
	pub fn parse(hash: &'a str) -> Option<Hash<'a>> {
		if hash.starts_with(APR1_ID) {
			let rest = hash.strip_prefix(APR1_ID)?;
			let (salt, digest) = rest.split_once('$')?;
			if salt.is_empty() || salt.len() > 8 || digest.is_empty() {
				return None;
			}
			Some(Hash::MD5(MD5Hash {
				salt: Cow::Borrowed(salt),
				hash: Cow::Borrowed(digest),
			}))
		} else if hash.starts_with(BCRYPT_ID) {
			bcrypt::HashParts::from_str(hash)
				.ok()
				.map(|_| Hash::BCrypt(Cow::Borrowed(hash)))
		} else if hash.starts_with("{SHA}") {
			Some(Hash::SHA1(Cow::Borrowed(&hash[SHA1_ID.len()..])))
		} else {
			// Ignore plaintext, assume crypt
			Some(Hash::Crypt(Cow::Borrowed(hash)))
		}
	}

	fn to_owned(&'a self) -> Hash<'static> {
		match self {
			Hash::MD5(MD5Hash { salt, hash }) => Hash::MD5(MD5Hash {
				salt: Cow::Owned(salt.to_string()),
				hash: Cow::Owned(hash.to_string()),
			}),
			Hash::BCrypt(hash) => Hash::BCrypt(Cow::Owned(hash.to_string())),
			Hash::SHA1(hash) => Hash::SHA1(Cow::Owned(hash.to_string())),
			Hash::Crypt(hash) => {
				let hash = hash.to_string();
				Hash::Crypt(Cow::Owned(hash))
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	static DATA: &str = "user2:$apr1$7/CTEZag$omWmIgXPJYoxB3joyuq4S/
user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00
bcrypt_test:$2y$05$nC6nErr9XZJuMJ57WyCob.EuZEjylDt2KaHfbfOtyb.EgL1I2jCVa
sha1_test:{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=
crypt_test:bGVh02xkuGli2";

	#[test]
	fn unix_crypt_verify_htpasswd() {
		let htpasswd = Htpasswd::new(DATA);
		assert_eq!(htpasswd.check("crypt_test", "password"), true);
	}

	#[test]
	fn sha1_verify_htpasswd() {
		let htpasswd = Htpasswd::new(DATA);
		assert_eq!(htpasswd.check("sha1_test", "password"), true);
	}

	#[test]
	fn bcrypt_verify_htpasswd() {
		let htpasswd = Htpasswd::new(DATA);
		assert_eq!(htpasswd.check("bcrypt_test", "password"), true);
	}

	#[test]
	fn md5_verify_htpasswd() {
		let htpasswd = Htpasswd::new(DATA);
		assert_eq!(htpasswd.check("user", "password"), true);
		assert_eq!(htpasswd.check("user", "passwort"), false);
		assert_eq!(htpasswd.check("user2", "zaq1@WSX"), true);
		assert_eq!(htpasswd.check("user2", "ZAQ1@WSX"), false);
	}

	#[test]
	fn md5_apr1() {
		assert_eq!(
			md5::format_hash(
				md5::md5_apr1_encode("password", "xxxxxxxx").as_str(),
				"xxxxxxxx",
			),
			"$apr1$xxxxxxxx$dxHfLAsjHkDRmG83UXe8K0".to_string()
		);
	}

	#[test]
	fn apr1() {
		assert!(md5::verify_apr1_hash("$apr1$xxxxxxxx$dxHfLAsjHkDRmG83UXe8K0", "password").unwrap());
	}

	#[test]
	fn malformed_apr1_verify_returns_error() {
		assert!(md5::verify_apr1_hash("$apr1$", "password").is_err());
	}

	#[test]
	fn user_not_found() {
		let htpasswd = Htpasswd::new(DATA);
		assert_eq!(htpasswd.check("user_does_not_exist", "password"), false);
	}

	#[test]
	fn malformed_apr1_entry_is_ignored() {
		let htpasswd = Htpasswd::new("broken:$apr1$\nuser:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00");
		assert!(!htpasswd.check("broken", "password"));
		assert!(htpasswd.check("user", "password"));
	}

	#[test]
	fn malformed_bcrypt_entry_is_ignored() {
		let htpasswd = Htpasswd::new(
			"broken:$2y$05$short\nbcrypt_test:$2y$05$nC6nErr9XZJuMJ57WyCob.EuZEjylDt2KaHfbfOtyb.EgL1I2jCVa",
		);
		assert!(!htpasswd.check("broken", "password"));
		assert!(htpasswd.check("bcrypt_test", "password"));
	}
}
