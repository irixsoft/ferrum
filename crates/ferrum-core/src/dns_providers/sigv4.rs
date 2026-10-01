use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

pub struct Signer<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

pub struct Request<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    pub payload: &'a [u8],
}

pub struct Signed {
    pub amz_date: String,
    pub authorization: String,
}

impl Signer<'_> {
    pub fn sign(&self, req: &Request<'_>, now: OffsetDateTime) -> Signed {
        let now = now.to_offset(time::UtcOffset::UTC);
        let amz_date = format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        let date = &amz_date[..8];

        let mut query: Vec<(String, String)> = req
            .query
            .iter()
            .map(|(k, v)| (encode(k, true), encode(v, true)))
            .collect();
        query.sort();
        let canonical_query = query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let signed_headers = "host;x-amz-date";
        let canonical_request = format!(
            "{}\n{}\n{}\nhost:{}\nx-amz-date:{}\n\n{}\n{}",
            req.method,
            encode(req.path, false),
            canonical_query,
            req.host,
            amz_date,
            signed_headers,
            hex(&Sha256::digest(req.payload))
        );

        let scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex(&Sha256::digest(canonical_request.as_bytes()))
        );
        let signature = hex(&hmac(&self.signing_key(date), string_to_sign.as_bytes()));

        Signed {
            authorization: format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                self.access_key_id
            ),
            amz_date,
        }
    }

    fn signing_key(&self, date: &str) -> Vec<u8> {
        let k_date = hmac(
            format!("AWS4{}", self.secret_access_key).as_bytes(),
            date.as_bytes(),
        );
        let k_region = hmac(&k_date, self.region.as_bytes());
        let k_service = hmac(&k_region, self.service.as_bytes());
        hmac(&k_service, b"aws4_request")
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn encode(s: &str, slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

    #[test]
    fn matches_the_aws_get_vanilla_vector() {
        let signer = Signer {
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: SECRET,
            region: "us-east-1",
            service: "service",
        };
        let signed = signer.sign(
            &Request {
                method: "GET",
                host: "example.amazonaws.com",
                path: "/",
                query: &[],
                payload: b"",
            },
            OffsetDateTime::from_unix_timestamp(1_440_938_160).unwrap(),
        );
        assert_eq!(signed.amz_date, "20150830T123600Z");
        assert_eq!(
            signed.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn derives_the_documented_signing_key() {
        let signer = Signer {
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: SECRET,
            region: "us-east-1",
            service: "iam",
        };
        assert_eq!(
            hex(&signer.signing_key("20150830")),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    #[test]
    fn query_values_are_encoded_and_sorted() {
        assert_eq!(encode("a b/c*", true), "a%20b%2Fc%2A");
        assert_eq!(
            encode("/2013-04-01/hostedzone", false),
            "/2013-04-01/hostedzone"
        );
    }
}
