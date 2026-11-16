import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { router, useLocalSearchParams } from 'expo-router';
import { ActivityIndicator, Alert, View } from 'react-native';

import { ListingForm, toInput } from '../../components/ListingForm';
import { Note } from '../../components/ui';
import type { ListingInput } from '../../lib/api';
import { useSession } from '../../lib/session';
import { useColors } from '../../lib/theme';

export default function EditListing() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const listingId = Number(id);
  const { api } = useSession();
  const c = useColors();
  const qc = useQueryClient();

  const listing = useQuery({ queryKey: ['listing', listingId], queryFn: () => api.getListing(listingId) });
  const done = async () => {
    await qc.invalidateQueries({ queryKey: ['listings'] });
    qc.removeQueries({ queryKey: ['listing', listingId] });
    router.back();
  };
  const save = useMutation({ mutationFn: (input: ListingInput) => api.updateListing(listingId, input), onSuccess: done });
  const remove = useMutation({ mutationFn: () => api.deleteListing(listingId), onSuccess: done });

  if (listing.error) {
    return (
      <View style={{ padding: 16 }}>
        <Note>{listing.error.message}</Note>
      </View>
    );
  }
  if (!listing.data) return <ActivityIndicator style={{ marginTop: 48 }} color={c.accent} />;

  const l = listing.data;
  return (
    <ListingForm
      initial={toInput(l)}
      dateHint={`${l.date_hijri} · ${l.date_gregorian}`}
      busy={save.isPending || remove.isPending}
      error={(save.error ?? remove.error)?.message}
      onSave={(input) => save.mutate(input)}
      onDelete={() =>
        Alert.alert('حذف هذا العرض نهائياً؟', 'يُحذف من جدولك ومن موقعك.', [
          { text: 'إلغاء', style: 'cancel' },
          { text: 'حذف', style: 'destructive', onPress: () => remove.mutate() },
        ])
      }
    />
  );
}
