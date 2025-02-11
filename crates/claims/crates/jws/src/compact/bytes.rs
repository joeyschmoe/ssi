use crate::{DecodeError, DecodedJWS, DecodedSigningBytes, Header, InvalidHeader, JWS};
pub use base64::DecodeError as Base64DecodeError;
use base64::Engine;
use ssi_claims_core::{ProofValidationError, ResolverProvider, Verification};
use ssi_jwk::JWKResolver;
use std::{borrow::Cow, ops::Deref};

/// JWS in compact serialized form.
#[repr(transparent)]
pub struct CompactJWS([u8]);

impl CompactJWS {
    pub fn new<T>(data: &T) -> Result<&Self, InvalidCompactJWS<&T>>
    where
        T: ?Sized + AsRef<[u8]>,
    {
        let bytes = data.as_ref();
        if Self::check(bytes) {
            Ok(unsafe { Self::new_unchecked(bytes) })
        } else {
            Err(InvalidCompactJWS(data))
        }
    }

    /// Creates a new compact JWS without checking the data.
    ///
    /// # Safety
    ///
    /// The input `data` must represent a valid compact JWS.
    pub unsafe fn new_unchecked(data: &[u8]) -> &Self {
        std::mem::transmute(data)
    }

    pub fn check(data: &[u8]) -> bool {
        enum State {
            Header,
            Payload,
            Signature,
        }

        let mut state = State::Header;

        for &b in data {
            match state {
                State::Header => match b {
                    b'.' => state = State::Payload,
                    b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'+' | b'/' => (),
                    _ => return false,
                },
                State::Payload => {
                    if b == b'.' {
                        state = State::Signature
                    }
                }
                State::Signature => (),
            }
        }

        matches!(state, State::Signature)
    }

    pub fn check_signing_bytes(data: &[u8]) -> bool {
        for &b in data {
            match b {
                b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'+' | b'/' => (),
                b'.' => return true,
                _ => return false,
            }
        }

        false
    }

    fn header_end(&self) -> usize {
        self.0.iter().position(|b| *b == b'.').unwrap()
    }

    fn signature_start(&self) -> usize {
        self.0.len() - self.0.iter().rev().position(|b| *b == b'.').unwrap()
    }

    fn payload_start(&self) -> usize {
        self.header_end() + 1
    }

    pub(crate) fn payload_end(&self) -> usize {
        self.signature_start() - 1
    }

    /// Returns the Base64 encoded header.
    pub fn header(&self) -> &[u8] {
        &self.0[..self.header_end()]
    }

    pub fn decode_header(&self) -> Result<Header, InvalidHeader> {
        Header::decode(self.header())
    }

    /// Returns the Base64 encoded payload.
    pub fn payload(&self) -> &[u8] {
        &self.0[self.payload_start()..self.payload_end()]
    }

    /// Decode the payload bytes.
    ///
    /// The header is necessary to know how the payload is encoded.
    pub fn decode_payload(&self, header: &Header) -> Result<Cow<[u8]>, Base64DecodeError> {
        if header.base64urlencode_payload.unwrap_or(true) {
            Ok(Cow::Owned(
                base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(self.payload())?,
            ))
        } else {
            Ok(Cow::Borrowed(self.payload()))
        }
    }

    /// Returns the Base64 encoded signature.
    pub fn signature(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.0[self.signature_start()..]) }
    }

    pub fn decode_signature(&self) -> Result<Vec<u8>, Base64DecodeError> {
        base64::prelude::BASE64_URL_SAFE_NO_PAD.decode(self.signature())
    }

    /// Decodes the entire JWS.
    pub fn decode(&self) -> Result<JWS<Cow<[u8]>>, DecodeError> {
        let header = self.decode_header().map_err(DecodeError::Header)?;
        let payload = self.decode_payload(&header).map_err(DecodeError::Payload)?;
        let signature = self.decode_signature().map_err(DecodeError::Signature)?;
        Ok(JWS::new(header, payload, signature.into()))
    }

    /// Decodes the entire JWS while preserving the signing bytes so they can
    /// be verified.
    pub fn to_decoded(&self) -> Result<DecodedJWS<Cow<[u8]>>, DecodeError> {
        let signing_bytes = self.signing_bytes().to_owned();
        let jws = self.decode()?;

        Ok(DecodedJWS::new(
            DecodedSigningBytes {
                bytes: signing_bytes,
                header: jws.header,
                payload: jws.payload,
            },
            jws.signature,
        ))
    }

    /// Returns the signing bytes.
    ///
    /// It is the concatenation of the Base64 encoded headers, a period '.' and
    /// the Base64 encoded payload.
    pub fn signing_bytes(&self) -> &[u8] {
        &self.0[..self.payload_end()]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Verify the JWS signature.
    ///
    /// This will only check the signature and not the validity of the decoded
    /// payload. For instance if the payload is a set of JWT claims, those
    /// claims will not be validated.
    ///
    /// To perform a more precise verification, specify use a specialized `T`
    /// instead of the default [`Vec<u8>`]
    ///
    /// The `params` argument provides all the verification parameters required
    /// to validate the claims and proof.
    ///
    /// # What verification parameters should I use?
    ///
    /// Any type that providing a `JWKResolver` through the `ResolverProvider`
    /// trait will be fine. Notable implementors are:
    /// - [`VerificationParameters`](ssi_claims_core::VerificationParameters):
    ///   A good default providing many other common verification parameters that
    ///   are not necessary here.
    /// - [`JWK`](ssi_jwk::JWK): allows you to put a JWK as `params`, which
    ///   will resolve into itself. Can be useful if you don't need key resolution
    ///   because you know in advance what key was used to sign the JWS.
    ///
    /// # Passing the parameters by reference
    ///
    /// If the validation traits are implemented for `P`, they will be
    /// implemented for `&P` as well. This means the parameters can be passed
    /// by move *or* by reference.
    pub async fn verify<V>(&self, params: V) -> Result<Verification, ProofValidationError>
    where
        V: ResolverProvider,
        V::Resolver: JWKResolver,
    {
        let jws = self.to_decoded().unwrap();
        jws.verify(params).await
    }
}

/// JWS in compact serialized form.
pub struct CompactJWSBuf(Vec<u8>);

impl CompactJWSBuf {
    pub fn new(bytes: Vec<u8>) -> Result<Self, InvalidCompactJWS<Vec<u8>>> {
        if CompactJWS::check(&bytes) {
            Ok(Self(bytes))
        } else {
            Err(InvalidCompactJWS(bytes))
        }
    }

    /// # Safety
    ///
    /// The input `bytes` must represent a valid compact JWS.
    pub unsafe fn new_unchecked(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn from_signing_bytes_and_signature(
        signing_bytes: Vec<u8>,
        signature: &[u8],
    ) -> Result<Self, InvalidCompactJWS<Vec<u8>>> {
        let mut bytes = signing_bytes;
        bytes.push(b'.');
        bytes.extend(signature.iter().copied());
        Self::new(bytes)
    }

    /// Creates a new detached JWS.
    ///
    /// Detached means the payload will not appear in the JWS.
    pub fn new_detached(header: Header, signature: &[u8]) -> Self {
        let mut bytes = header.encode().into_bytes();
        bytes.extend([b'.', b'.']);
        bytes.extend(signature.iter().copied());
        unsafe { Self::new_unchecked(bytes) }
    }

    /// # Safety
    ///
    /// The input `signing_bytes` and `signature` must form a valid compact JWS
    /// once concatenated with a `.`.
    pub unsafe fn from_signing_bytes_and_signature_unchecked(
        signing_bytes: Vec<u8>,
        signature: Vec<u8>,
    ) -> Self {
        let mut bytes = signing_bytes;
        bytes.push(b'.');
        bytes.extend(signature);
        Self::new_unchecked(bytes)
    }

    pub fn as_compact_jws(&self) -> &CompactJWS {
        unsafe { CompactJWS::new_unchecked(&self.0) }
    }

    pub fn into_signing_bytes(mut self) -> Vec<u8> {
        self.0.truncate(self.payload_end()); // remove the signature.
        self.0
    }

    /// Decodes the entire JWS while preserving the signing bytes so they can
    /// be verified.
    pub fn into_decoded(self) -> Result<DecodedJWS<Vec<u8>>, DecodeError> {
        let decoded = self.decode()?.into_owned();
        let signing_bytes = self.into_signing_bytes();
        Ok(DecodedJWS::new(
            DecodedSigningBytes::new(signing_bytes, decoded.header, decoded.payload),
            decoded.signature,
        ))
    }
}

impl Deref for CompactJWSBuf {
    type Target = CompactJWS;

    fn deref(&self) -> &Self::Target {
        self.as_compact_jws()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid compact JWS")]
pub struct InvalidCompactJWS<B = String>(pub B);

impl<'a> InvalidCompactJWS<&'a [u8]> {
    pub fn into_owned(self) -> InvalidCompactJWS<Vec<u8>> {
        InvalidCompactJWS(self.0.to_owned())
    }
}
