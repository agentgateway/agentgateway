/*
 * This work is derived from Apache Software Foundation's http server project
 *
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/*
 * The apr_md5_encode() routine uses much code obtained from the FreeBSD 3.0
 * MD5 crypt() function, which is licenced as follows:
 * ----------------------------------------------------------------------------
 * "THE BEER-WARE LICENSE" (Revision 42):
 * <phk@login.dknet.dk> wrote this file.  As long as you retain this notice you
 * can do whatever you want with this stuff. If we meet some day, and you think
 * this stuff is worth it, you can buy me a beer in return.   Poul-Henning Kamp
 * ----------------------------------------------------------------------------
 */

use md5::{Digest, Md5};

pub(crate) const APR1_ID: &str = "$apr1$";

fn encode_digest(digest: &[u32; 16]) -> String {
	let mut p = vec![0u8; 22];
	let l = ((digest[0] << 16) | (digest[6] << 8) | digest[12]) as u64;
	to_64(&mut p[0..4], l);

	let l = ((digest[1] << 16) | (digest[7] << 8) | digest[13]) as u64;
	to_64(&mut p[4..8], l);

	let l = ((digest[2] << 16) | (digest[8] << 8) | digest[14]) as u64;
	to_64(&mut p[8..12], l);

	let l = ((digest[3] << 16) | (digest[9] << 8) | digest[15]) as u64;
	to_64(&mut p[12..16], l);

	let l = ((digest[4] << 16) | (digest[10] << 8) | digest[5]) as u64;
	to_64(&mut p[16..20], l);

	let l = digest[11] as u64;
	to_64(&mut p[20..22], l);

	String::from_utf8(p).unwrap()
}

fn to_64(s: &mut [u8], mut v: u64) {
	let itoa64 = "./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".as_bytes();

	for b in s.iter_mut() {
		*b = itoa64[(v & 0x3f) as usize];
		v >>= 6;
	}
}

/// Calculates apache specific md5 hash
/// Returns just the hashed password, use [format_hash](fn.format_hash.html) to get the hash in htpasswd format
pub fn md5_apr1_encode(pw: &str, salt: &str) -> String {
	let mut sp = salt.as_bytes();
	let pw = pw.as_bytes();

	if sp.starts_with(APR1_ID.as_bytes()) {
		sp = &sp[APR1_ID.len()..sp.len()];
	}

	let mut ctx = Md5::new();
	ctx.update(pw);
	ctx.update(APR1_ID.as_bytes());
	ctx.update(sp);

	let mut ctx1 = Md5::new();
	ctx1.update(pw);
	ctx1.update(sp);
	ctx1.update(pw);

	let mut digest = ctx1.finalize_reset();

	for pl in (0..pw.len()).rev().step_by(Md5::output_size()) {
		let digest_pl = pl.min(Md5::output_size()) + 1;
		ctx.update(&digest[..digest_pl]);
	}

	let mut i = pw.len();
	while i != 0 {
		if i & 1 != 0 {
			ctx.update([0]);
		} else {
			ctx.update(&pw[..1]);
		}
		i >>= 1;
	}

	digest = ctx.finalize();

	for i in 0u32..1000u32 {
		if i & 1 != 0 {
			ctx1.update(pw);
		} else {
			ctx1.update(digest);
		}
		if i % 3 != 0 {
			ctx1.update(sp);
		}

		if i % 7 != 0 {
			ctx1.update(pw);
		}

		if i & 1 != 0 {
			ctx1.update(digest);
		} else {
			ctx1.update(pw);
		}
		ctx1.finalize_into_reset(&mut digest);
	}

	let mut digest_final: [u32; 16] = [0; 16];
	digest
		.iter()
		.enumerate()
		.for_each(|(idx, &x)| digest_final[idx] = x as u32);

	encode_digest(&digest_final)
}

pub fn format_hash(password: &str, salt: &str) -> String {
	format!("{}{}${}", APR1_ID, salt, password)
}

/// Assumes the hash is in the correct format - $apr1$salt$password
pub fn verify_apr1_hash(hash: &str, password: &str) -> Result<bool, &'static str> {
	let salt = &hash[6..14];
	Ok(format_hash(&md5_apr1_encode(password, salt), salt) == hash)
}
