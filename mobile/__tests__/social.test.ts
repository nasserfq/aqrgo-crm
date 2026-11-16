// Google sign-in's return URL handling.
jest.mock('expo-web-browser', () => ({ openAuthSessionAsync: jest.fn() }));
jest.mock('expo-crypto', () => ({
  getRandomBytes: () => new Uint8Array(32),
  digestStringAsync: async () => 'ab'.repeat(32),
  CryptoDigestAlgorithm: { SHA256: 'SHA-256' },
  CryptoEncoding: { HEX: 'hex' },
}));
jest.mock('expo-apple-authentication', () => ({}));

import * as WebBrowser from 'expo-web-browser';

import type { Api } from '../src/lib/api';
import { signInWithGoogle } from '../src/lib/social';

it('ignores the empty fragment iOS leaves on the return URL', async () => {
  (WebBrowser.openAuthSessionAsync as jest.Mock).mockResolvedValue({
    type: 'success',
    url: 'aqrgo://auth?code=abc123#',
  });
  const googleSignIn = jest.fn(async () => ({ token: 't', user: { id: 1, email: 'a@b.c', name: '' } }));
  await signInWithGoogle({ googleSignIn } as unknown as Api, 'https://aqrgo.com');
  expect(googleSignIn).toHaveBeenCalledWith('abc123', expect.any(String));
});
