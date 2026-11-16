// Sign in: Google, Apple (on iPhone), or email then the 6-digit code from the
// email (same as the web).
import { useEffect, useState } from 'react';
import * as AppleAuthentication from 'expo-apple-authentication';
import { Image, KeyboardAvoidingView, Platform, ScrollView, useColorScheme, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Button, Field, Note, T } from '../components/ui';
import { ApiError } from '../lib/api';
import { API_URL, useSession } from '../lib/session';
import { appleAvailable, signInWithApple, signInWithGoogle, type SignedIn } from '../lib/social';
import { fonts, radius, useColors } from '../lib/theme';

export default function Login() {
  const { api, signIn } = useSession();
  const c = useColors();
  const scheme = useColorScheme();
  const insets = useSafeAreaInsets();
  const [step, setStep] = useState<'email' | 'code'>('email');
  const [email, setEmail] = useState('');
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [google, setGoogle] = useState(false);
  const [apple, setApple] = useState(false);

  useEffect(() => {
    api.authOptions().then((o) => setGoogle(o.google), () => setGoogle(false));
    if (Platform.OS === 'ios') appleAvailable().then(setApple);
  }, [api]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'حدث خطأ، حاول مرة أخرى');
    } finally {
      setBusy(false);
    }
  };

  const sendCode = () =>
    run(async () => {
      await api.requestCode(email.trim());
      setCode('');
      setStep('code');
    });

  const withProvider = (start: () => Promise<SignedIn>) =>
    run(async () => {
      const signed = await start();
      if (signed) await signIn(signed.token, signed.user);
    });

  const verify = () =>
    run(async () => {
      const { token, user } = await api.verifyCode(email.trim(), code);
      await signIn(token, user);
    });

  return (
    <KeyboardAvoidingView style={{ flex: 1, backgroundColor: c.bg }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={{ padding: 24, paddingTop: insets.top + 48, gap: 20, flexGrow: 1 }}>
        <Image source={require('../../assets/logo.png')} style={{ width: 211, height: 58 }} resizeMode="contain" accessibilityLabel="AQRGO" />
        <T style={{ fontFamily: fonts.display, fontSize: 30, lineHeight: 46 }}>
          الصق العرض.{'\n'}
          <T style={{ fontFamily: fonts.display, fontSize: 30, color: c.accent }}>والجدول يرتب نفسه.</T>
        </T>
        <T style={{ color: c.muted }}>جدول العروض العقارية، يُعبأ من التطبيق أو الموقع أو من تيليجرام</T>

        {error ? <Note>{error}</Note> : null}

        {step === 'email' ? (
          <View style={{ gap: 16 }}>
            {apple ? (
              <AppleAuthentication.AppleAuthenticationButton
                testID="apple"
                buttonType={AppleAuthentication.AppleAuthenticationButtonType.SIGN_IN}
                buttonStyle={
                  scheme === 'dark'
                    ? AppleAuthentication.AppleAuthenticationButtonStyle.WHITE
                    : AppleAuthentication.AppleAuthenticationButtonStyle.BLACK
                }
                cornerRadius={radius}
                style={{ height: 52 }}
                onPress={() => withProvider(() => signInWithApple(api))}
              />
            ) : null}
            {google ? (
              <Button
                testID="google"
                kind="outline"
                label="الدخول بحساب Google"
                onPress={() => withProvider(() => signInWithGoogle(api, API_URL))}
                disabled={busy}
              />
            ) : null}
            {apple || google ? <T style={{ color: c.muted, textAlign: 'center' }}>أو برمز على بريدك</T> : null}
            <Field
              testID="email"
              label="البريد الإلكتروني"
              placeholder="name@example.com"
              value={email}
              onChangeText={setEmail}
              autoCapitalize="none"
              autoComplete="email"
              keyboardType="email-address"
              textContentType="emailAddress"
              returnKeyType="send"
              onSubmitEditing={sendCode}
            />
            <Button testID="send-code" label="أرسل رمز الدخول" onPress={sendCode} busy={busy} disabled={!email.trim()} />
          </View>
        ) : (
          <View style={{ gap: 16 }}>
            <Note kind="info">أرسلنا رمزاً من ٦ أرقام إلى {email.trim()}</Note>
            <Field
              testID="code"
              label="رمز الدخول"
              placeholder="••••••"
              value={code}
              onChangeText={setCode}
              keyboardType="number-pad"
              textContentType="oneTimeCode"
              autoComplete="one-time-code"
              // Real codes are 6 digits; App Review's demo account uses a longer one.
              maxLength={16}
              autoFocus
              onSubmitEditing={verify}
              style={{
                fontSize: 26,
                letterSpacing: code.length > 6 ? 2 : 10,
                textAlign: 'center',
                fontFamily: fonts.display,
              }}
            />
            <Button testID="verify" label="دخول" onPress={verify} busy={busy} disabled={code.length < 6} />
            <Button label="تغيير البريد" kind="outline" onPress={() => setStep('email')} />
          </View>
        )}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}
