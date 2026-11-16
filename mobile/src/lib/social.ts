// Google and Apple sign-in for the phone.
//
// Google: the app opens aqrgo.com/auth/google in the system sign-in sheet with
// a challenge (SHA-256 of a random verifier it keeps). The server sends the
// sheet back to aqrgo://auth?code=… and the app trades code + verifier for its
// token, so a code caught by another app is useless (docs/api.md).
//
// Apple: the system sheet gives an identity token signed by Apple, which the
// server checks.
import * as AppleAuthentication from 'expo-apple-authentication';
import * as Crypto from 'expo-crypto';
import * as WebBrowser from 'expo-web-browser';

import { ApiError, type Api, type User } from './api';

const RETURN_URL = 'aqrgo://auth';
const GOOGLE_FAILED = 'تعذر الدخول عبر Google، حاول مرة أخرى';
const NOT_ALLOWED = 'هذا البريد غير مسموح له بالدخول';

/** null when the person closed the sheet. */
export type SignedIn = { token: string; user: User } | null;

function toHex(bytes: Uint8Array) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export async function signInWithGoogle(api: Api, baseUrl: string): Promise<SignedIn> {
  const verifier = toHex(Crypto.getRandomBytes(32));
  const challenge = (
    await Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, verifier, {
      encoding: Crypto.CryptoEncoding.HEX,
    })
  ).toLowerCase();
  const result = await WebBrowser.openAuthSessionAsync(
    `${baseUrl.replace(/\/+$/, '')}/auth/google?app=${challenge}`,
    RETURN_URL,
    { preferEphemeralSession: false },
  );
  if (result.type !== 'success') return null;
  const query = result.url.split('?')[1] ?? '';
  const params = new Map(
    query.split('&').map((kv) => {
      const [k, v = ''] = kv.split('=');
      return [decodeURIComponent(k), decodeURIComponent(v)] as const;
    }),
  );
  const error = params.get('error');
  if (error) throw new ApiError(401, error === 'not_allowed' ? NOT_ALLOWED : GOOGLE_FAILED);
  const code = params.get('code');
  if (!code) throw new ApiError(401, GOOGLE_FAILED);
  return api.googleSignIn(code, verifier);
}

export const appleAvailable = () => AppleAuthentication.isAvailableAsync().catch(() => false);

export async function signInWithApple(api: Api): Promise<SignedIn> {
  let credential: AppleAuthentication.AppleAuthenticationCredential;
  try {
    credential = await AppleAuthentication.signInAsync({
      requestedScopes: [
        AppleAuthentication.AppleAuthenticationScope.FULL_NAME,
        AppleAuthentication.AppleAuthenticationScope.EMAIL,
      ],
    });
  } catch (e) {
    if ((e as { code?: string }).code === 'ERR_REQUEST_CANCELED') return null;
    throw new ApiError(401, 'تعذر الدخول عبر Apple، حاول مرة أخرى');
  }
  if (!credential.identityToken) throw new ApiError(401, 'تعذر الدخول عبر Apple، حاول مرة أخرى');
  const name = [credential.fullName?.givenName, credential.fullName?.familyName]
    .filter(Boolean)
    .join(' ');
  return api.appleSignIn(credential.identityToken, name);
}
