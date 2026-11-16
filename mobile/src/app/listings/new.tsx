import { useMutation, useQueryClient } from '@tanstack/react-query';
import { router } from 'expo-router';

import { ListingForm } from '../../components/ListingForm';
import type { ListingInput } from '../../lib/api';
import { useSession } from '../../lib/session';

export default function NewListing() {
  const { api } = useSession();
  const qc = useQueryClient();
  const save = useMutation({
    mutationFn: (input: ListingInput) => api.createListing(input),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ['listings'] });
      router.back();
    },
  });
  return (
    <ListingForm
      initial={{ status: 'available' }}
      busy={save.isPending}
      error={save.error?.message}
      onSave={(input) => save.mutate(input)}
    />
  );
}
