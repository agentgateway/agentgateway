# htpasswd-verify

Verify apache's htpasswd file

Supports MD5, BCrypt, SHA1, Unix crypt

# Examples

Verify MD5 hash

```
let data = "user:$apr1$lZL6V/ci$eIMz/iKDkbtys/uU7LEK00";
let htpasswd = htpasswd_verify::Htpasswd::from(data);
assert!(htpasswd.check("user", "password"));
```

It also allows to encrypt with md5 (not the actual md5, but the apache specific md5 that
htpasswd file uses)

```
use htpasswd_verify::md5::{md5_apr1_encode, format_hash};

let password = "password";
let hash = md5_apr1_encode(password, "RandSalt");
let hash = format_hash(&hash, "RandSalt");
assert_eq!(hash, "$apr1$RandSalt$PgCXHRrkpSt4cbyC2C6bm/");
```