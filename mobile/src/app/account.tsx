// The account screen: who is signed in, sign out, privacy, and deleting the
// account (App Store rules require deletion inside the app).
import { useState } from 'react';
import { Alert, Linking, ScrollView, View } from 'react-native';

import { Button, Note, T } from '../components/ui';
import { ApiError } from '../lib/api';
import { API_URL, useSession } from '../lib/session';
import { fonts, useColors } from '../lib/theme';

export default function Account() {
  const { api, user, signOut, forget } = useSession();
  const c = useColors();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
