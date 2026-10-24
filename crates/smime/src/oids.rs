//! The object identifiers S/MIME is written with (RFC 5652, 8551, 5280, 3447/8017, 5480,
//! 3565, 5084).

use der::asn1::ObjectIdentifier as Oid;

// Content types.
pub const DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.1");
pub const SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
pub const ENVELOPED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.3");
pub const AUTH_ENVELOPED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.1.23");

// Signed attributes.
pub const CONTENT_TYPE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.3");
pub const MESSAGE_DIGEST: Oid = Oid::new_unwrap("1.2.840.113549.1.9.4");
pub const SIGNING_TIME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.5");
pub const SMIME_CAPABILITIES: Oid = Oid::new_unwrap("1.2.840.113549.1.9.15");

// Keys and signatures.
pub const RSA_ENCRYPTION: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
pub const RSAES_OAEP: Oid = Oid::new_unwrap("1.2.840.113549.1.1.7");
pub const RSASSA_PSS: Oid = Oid::new_unwrap("1.2.840.113549.1.1.10");
pub const SHA1_WITH_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
pub const SHA256_WITH_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
pub const SHA384_WITH_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
pub const SHA512_WITH_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
pub const EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
pub const ECDSA_WITH_SHA1: Oid = Oid::new_unwrap("1.2.840.10045.4.1");
pub const ECDSA_WITH_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
pub const ECDSA_WITH_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
pub const ECDSA_WITH_SHA512: Oid = Oid::new_unwrap("1.2.840.10045.4.3.4");
pub const P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
pub const P384: Oid = Oid::new_unwrap("1.3.132.0.34");

// Digests.
pub const SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
pub const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
pub const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
pub const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");

// Content encryption.
pub const AES128_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.2");
pub const AES192_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.22");
pub const AES256_CBC: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.42");
pub const AES128_GCM: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.6");
pub const AES192_GCM: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.26");
pub const AES256_GCM: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.1.46");
pub const DES_EDE3_CBC: Oid = Oid::new_unwrap("1.2.840.113549.3.7");

// Names and extensions.
pub const COMMON_NAME: Oid = Oid::new_unwrap("2.5.4.3");
pub const ORGANIZATION: Oid = Oid::new_unwrap("2.5.4.10");
pub const EMAIL_ADDRESS: Oid = Oid::new_unwrap("1.2.840.113549.1.9.1");
pub const SUBJECT_KEY_ID: Oid = Oid::new_unwrap("2.5.29.14");
pub const KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.15");
pub const SUBJECT_ALT_NAME: Oid = Oid::new_unwrap("2.5.29.17");
pub const BASIC_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.19");
pub const EXT_KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.37");
pub const ANY_EXTENDED_KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.37.0");
pub const EMAIL_PROTECTION: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.4");
