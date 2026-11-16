// Add a row: paste the broker's message for the AI to fill the form, or type it.
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { router } from 'expo-router';
import { useState } from 'react';
import { View } from 'react-native';

import { ListingForm } from '../../components/ListingForm';
import { Button, Field, Note, T } from '../../components/ui';
import { ApiError, type ListingDraft, type ListingInput } from '../../lib/api';
import { useSession } from '../../lib/session';
import { fonts, radius, useColors } from '../../lib/theme';

function fromDraft(d: ListingDraft): ListingInput {
  const input: ListingInput = { ...d, status: 'available', listed_on: d.listed_on ?? '' };
  return input;
}

export default function NewListing() {
  const { api } = useSession();
  const qc = useQueryClient();
  const c = useColors();
  const [initial, setInitial] = useState<ListingInput>({ status: 'available' });
  const [formKey, setFormKey] = useState(0);
  const [text, setText] = useState('');
  const [reading, setReading] = useState(false);
  const [note, setNote] = useState<{ kind: 'info' | 'error'; text: string } | null>(null);

  const save = useMutation({
    mutationFn: (input: ListingInput) => api.createListing(input),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ['listings'] });
      router.back();
    },
  });

  const read = async () => {
    setReading(true);
    setNote(null);
    try {
      const drafts = await api.extractListings(text);
      if (drafts.length === 0) {
        setNote({ kind: 'error', text: 'لم أجد عرضاً عقارياً في النص. عبّئ الخانات يدوياً.' });
        return;
      }
      setInitial(fromDraft(drafts[0]));
      setFormKey((k) => k + 1);
      setNote({
        kind: 'info',
        text:
          drafts.length > 1
            ? `وجدت ${drafts.length} عروض في النص وعبّأت الأول. راجع الخانات ثم احفظ.`
            : 'عبّأت الخانات من النص. راجعها ثم احفظ.',
      });
    } catch (e) {
      setNote({ kind: 'error', text: e instanceof ApiError ? e.message : 'حدث خطأ، حاول مرة أخرى' });
    } finally {
      setReading(false);
    }
  };

  const paste = (
    <View style={{ gap: 12, backgroundColor: c.accentSoft, borderRadius: radius, padding: 16 }}>
      <T style={{ fontFamily: fonts.display, fontSize: 17, color: c.accentDeep }}>الصق العرض</T>
      <Field
        testID="paste"
        placeholder="الصق رسالة الوسيط كما هي"
        value={text}
        onChangeText={setText}
        multiline
      />
      <Button
        testID="read"
        label="رتّبها بالذكاء الاصطناعي"
        onPress={() => void read()}
        busy={reading}
        disabled={!text.trim()}
      />
      {note ? <Note kind={note.kind}>{note.text}</Note> : null}
    </View>
  );

  return (
    <ListingForm
      key={formKey}
      header={paste}
      initial={initial}
      busy={save.isPending}
      error={save.error?.message}
      onSave={(input) => save.mutate(input)}
    />
  );
}
