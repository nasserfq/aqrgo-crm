// The table: search, status filter, one card per row, newest first.
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { router, Stack } from 'expo-router';
import { useEffect, useState } from 'react';
import { FlatList, Pressable, RefreshControl, ScrollView, View } from 'react-native';

import { ListingCard } from '../components/ListingCard';
import { Button, Field, Note, T } from '../components/ui';
import type { Status } from '../lib/api';
import { useSession } from '../lib/session';
import { fonts, radius, useColors } from '../lib/theme';

const FILTERS: { value: Status | ''; label: string }[] = [
  { value: '', label: 'كل الحالات' },
  { value: 'available', label: 'متاح' },
  { value: 'reserved', label: 'محجوز' },
  { value: 'sold', label: 'مباع' },
];

function useDebounced<T>(value: T, ms: number) {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export default function Listings() {
  const { api } = useSession();
  const c = useColors();
  const [q, setQ] = useState('');
  const [status, setStatus] = useState<Status | ''>('');
  const query = useDebounced(q, 300);

  const listings = useQuery({
    queryKey: ['listings', query, status],
    queryFn: () => api.listListings({ q: query, status }),
    placeholderData: keepPreviousData,
  });

  return (
    <View style={{ flex: 1 }}>
      <Stack.Screen
        options={{
          headerRight: () => (
            <Pressable
              testID="account"
              accessibilityRole="button"
              onPress={() => router.push('/account')}
              hitSlop={12}
              style={{ paddingHorizontal: 8 }}>
              <T style={{ color: c.accent, fontFamily: fonts.medium }}>حسابي</T>
            </Pressable>
          ),
        }}
      />
      <FlatList
        data={listings.data ?? []}
        keyExtractor={(l) => String(l.id)}
        contentContainerStyle={{ padding: 16, gap: 12, paddingBottom: 48 }}
        keyboardShouldPersistTaps="handled"
        refreshControl={
          <RefreshControl refreshing={listings.isRefetching} onRefresh={() => void listings.refetch()} tintColor={c.accent} />
        }
        ListHeaderComponent={
          <View style={{ gap: 12, marginBottom: 4 }}>
            <Button testID="add" label="+ إضافة عرض" onPress={() => router.push('/listings/new')} />
            <Field testID="search" placeholder="ابحث في عروضك" value={q} onChangeText={setQ} returnKeyType="search" clearButtonMode="while-editing" />
            <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: 8 }}>
              {FILTERS.map((f) => {
                const on = f.value === status;
                return (
                  <Pressable
                    key={f.label}
                    testID={`filter-${f.value || 'all'}`}
                    accessibilityRole="button"
                    accessibilityState={{ selected: on }}
                    onPress={() => setStatus(f.value)}
                    style={{
                      paddingHorizontal: 14,
                      paddingVertical: 8,
                      borderRadius: radius,
                      borderWidth: 1,
                      borderColor: on ? c.accent : c.lineStrong,
                      backgroundColor: on ? c.accentSoft : c.surface,
                    }}>
                    <T style={{ color: on ? c.accentDeep : c.ink, fontFamily: on ? fonts.bold : fonts.body }}>{f.label}</T>
                  </Pressable>
                );
              })}
            </ScrollView>
            {listings.data ? <T style={{ color: c.muted }}>العروض: {listings.data.length}</T> : null}
            {listings.error ? <Note>{(listings.error as Error).message}</Note> : null}
          </View>
        }
        ListEmptyComponent={
          listings.isLoading ? undefined : (
            <View style={{ alignItems: 'center', padding: 40, gap: 6 }}>
              <T style={{ fontFamily: fonts.display, fontSize: 18 }}>{q || status ? 'لا نتائج' : 'لا توجد عروض بعد'}</T>
              <T style={{ color: c.muted, textAlign: 'center' }}>
                {q || status ? 'جرّب كلمة أخرى أو حالة أخرى' : 'أضف أول عرض، أو أرسله إلى بوت تيليجرام'}
              </T>
            </View>
          )
        }
        renderItem={({ item }) => (
          <ListingCard listing={item} onPress={() => router.push({ pathname: '/listings/[id]', params: { id: String(item.id) } })} />
        )}
      />
    </View>
  );
}
