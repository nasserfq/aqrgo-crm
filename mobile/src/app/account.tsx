// The account screen: who is signed in, linking the Telegram and WhatsApp
// bots, sign out, privacy, and deleting the account (App Store rules require
// deletion inside the app).
import { useCallback, useState } from 'react';
import { Alert, AppState, Linking, ScrollView, View } from 'react-native';
import { useFocusEffect } from 'expo-router';

import { Button, Note, T } from '../components/ui';
import { ApiError, type ChannelName, type Channels } from '../lib/api';
import { API_URL, useSession } from '../lib/session';
import { fonts, useColors } from '../lib/theme';

export default function Account() {
  const { api, user, signOut, forget } = useSession();
  const c = useColors();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [channels, setChannels] = useState<Channels | null>(null);

  const refresh = useCallback(() => {
    api.channels().then(setChannels, () => setChannels(null));
  }, [api]);

  // Linking finishes in Telegram or WhatsApp, so check again on coming back.
  useFocusEffect(
    useCallback(() => {
      refresh();
      const sub = AppState.addEventListener('change', (s) => s === 'active' && refresh());
      return () => sub.remove();
    }, [refresh]),
  );

  const link = async (channel: ChannelName) => {
    setError(null);
    try {
      await Linking.openURL(await api.linkChannel(channel));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'تعذر فتح التطبيق، حاول مرة أخرى');
    }
  };

  const unlink = async (channel: ChannelName) => {
    setError(null);
    try {
      await api.unlinkChannel(channel);
      refresh();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'حدث خطأ، حاول مرة أخرى');
    }
  };

  const deleteAccount = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.deleteAccount();
      await forget();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'حدث خطأ، حاول مرة أخرى');
      setBusy(false);
    }
  };

  const confirmDelete = () =>
    Alert.alert(
      'حذف الحساب',
      'سيُحذف حسابك وكل عروضك وموقعك نهائياً، ولا يمكن استرجاعها.',
      [
        { text: 'إلغاء', style: 'cancel' },
        { text: 'احذف نهائياً', style: 'destructive', onPress: () => void deleteAccount() },
      ],
    );

  return (
    <ScrollView contentContainerStyle={{ padding: 16, gap: 16 }}>
      <View style={{ gap: 4 }}>
        {user?.name ? <T style={{ fontFamily: fonts.bold, fontSize: 18 }}>{user.name}</T> : null}
        <T style={{ color: c.muted }} selectable>
          {user?.email}
        </T>
      </View>
      {error ? <Note>{error}</Note> : null}
      {CHANNELS.filter(({ key }) => channels?.[key].available).map(({ key, name, hint }) =>
        channels?.[key].linked ? (
          <View key={key} style={{ gap: 8 }}>
            <T style={{ fontFamily: fonts.bold }}>{`${name} مربوط`}</T>
            <T style={{ color: c.muted, fontSize: 14 }}>{hint}</T>
            <Button testID={`unlink-${key}`} kind="outline" label={`فك ربط ${name}`} onPress={() => void unlink(key)} />
          </View>
        ) : (
          <View key={key} style={{ gap: 8 }}>
            <Button testID={`link-${key}`} label={`ربط ${name}`} onPress={() => void link(key)} />
            <T style={{ color: c.muted, fontSize: 14 }}>{`يفتح ${name} لتربط حسابك، ثم أرسل العروض للبوت وتصل هنا.`}</T>
          </View>
        ),
      )}
      <Button testID="sign-out" kind="outline" label="تسجيل الخروج" onPress={() => void signOut()} disabled={busy} />
      <Button
        testID="privacy"
        kind="outline"
        label="سياسة الخصوصية"
        onPress={() => void Linking.openURL(`${API_URL}/privacy`)}
      />
      <View style={{ height: 24 }} />
      <Button testID="delete-account" kind="danger" label="حذف الحساب" onPress={confirmDelete} busy={busy} />
      <T style={{ color: c.muted, fontSize: 14 }}>يحذف حسابك وكل عروضك والجوالات المسجلة وموقعك العام.</T>
    </ScrollView>
  );
}

const CHANNELS: { key: ChannelName; name: string; hint: string }[] = [
  { key: 'telegram', name: 'تيليجرام', hint: 'أرسل العروض لبوت تيليجرام وتُحفظ في حسابك.' },
  { key: 'whatsapp', name: 'واتساب', hint: 'أرسل العروض لرقم واتساب AQRGO وتُحفظ في حسابك.' },
];
