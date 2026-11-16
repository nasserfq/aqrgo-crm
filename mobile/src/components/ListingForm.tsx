// The add / edit form, grouped like the web form.
import { useState } from 'react';
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, View } from 'react-native';

import type { Listing, ListingInput, Status } from '../lib/api';
import { fonts, radius, useColors } from '../lib/theme';
import { Button, Field, Note, T } from './ui';

type Key = Exclude<keyof ListingInput, 'status' | 'listed_on'>;

const LABELS: Record<Key, string> = {
  offer_type: 'نوع العرض',
  location: 'الموقع',
  plan_no: 'رقم المخطط',
  plot_no: 'رقم القطعة',
  area: 'المساحة',
  price: 'السعر',
  frontages: 'الواجهات والشوارع',
  seller: 'بيانات البائع',
  buyer: 'بيانات المشتري',
  broker: 'بيانات الوسيط',
  rooms: 'عدد الغرف',
  bathrooms: 'دورات المياه',
  floors: 'الأدوار',
  units: 'الوحدات',
  income: 'الدخل والعقود',
  details: 'تفاصيل إضافية',
};

const SECTIONS: { title: string; keys: Key[] }[] = [
  { title: 'العقار', keys: ['offer_type', 'location', 'plan_no', 'plot_no', 'area', 'price', 'frontages'] },
  { title: 'الأطراف', keys: ['seller', 'buyer', 'broker'] },
  { title: 'تفاصيل المبنى', keys: ['rooms', 'bathrooms', 'floors', 'units', 'income', 'details'] },
];

const MULTILINE = new Set<Key>(['frontages', 'buyer', 'seller', 'broker', 'income', 'details']);
const NUMERIC = new Set<Key>(['plan_no', 'plot_no', 'area', 'rooms', 'bathrooms', 'floors', 'units']);

export const OFFER_TYPES = ['أرض للبيع', 'فيلا للبيع', 'شقة للبيع', 'عمارة للبيع', 'دبلكس للبيع', 'أرض للإيجار', 'فيلا للإيجار', 'شقة للإيجار', 'محل للإيجار', 'طلب شراء'];

const STATUSES: { value: Status; label: string }[] = [
  { value: 'available', label: 'متاح' },
  { value: 'reserved', label: 'محجوز' },
  { value: 'sold', label: 'مباع' },
];

export function toInput(l: Listing): ListingInput {
  const input: ListingInput = { status: l.status, listed_on: l.listed_on };
  for (const k of Object.keys(LABELS) as Key[]) input[k] = l[k];
  return input;
}

function Chip({ label, on, onPress, testID }: { label: string; on: boolean; onPress: () => void; testID?: string }) {
  const c = useColors();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityState={{ selected: on }}
      onPress={onPress}
      style={{
        paddingHorizontal: 12,
        paddingVertical: 6,
        borderRadius: radius,
        borderWidth: 1,
        borderColor: on ? c.accent : c.lineStrong,
        backgroundColor: on ? c.accentSoft : c.surface,
      }}>
      <T style={{ fontSize: 14, color: on ? c.accentDeep : c.ink, fontFamily: on ? fonts.bold : fonts.body }}>{label}</T>
    </Pressable>
  );
}

type Props = {
  initial: ListingInput;
  dateHint?: string;
  error?: string | null;
  busy?: boolean;
  onSave: (input: ListingInput) => void;
  onDelete?: () => void;
};

export function ListingForm({ initial, dateHint, error, busy, onSave, onDelete }: Props) {
  const c = useColors();
  const [v, setV] = useState<ListingInput>(initial);
  const set = (k: keyof ListingInput, value: string) => setV((old) => ({ ...old, [k]: value }));

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={{ padding: 16, gap: 20, paddingBottom: 48 }}>
        {error ? <Note>{error}</Note> : null}
        {SECTIONS.map((s) => (
          <View key={s.title} style={{ gap: 14, backgroundColor: c.surface, borderColor: c.line, borderWidth: 1, borderRadius: radius, padding: 16 }}>
            <T style={{ fontFamily: fonts.display, fontSize: 17, color: c.accentDeep }}>{s.title}</T>
            {s.keys.map((k) => (
              <View key={k} style={{ gap: 8 }}>
                <Field
                  testID={`field-${k}`}
                  label={LABELS[k]}
                  value={v[k] ?? ''}
                  onChangeText={(t) => set(k, t)}
                  multiline={MULTILINE.has(k)}
                  keyboardType={NUMERIC.has(k) ? 'numbers-and-punctuation' : 'default'}
                  placeholder={k === 'offer_type' ? 'مثال: أرض للبيع' : k === 'location' ? 'الحي، المدينة' : undefined}
                />
                {k === 'offer_type' ? (
                  <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: 8 }}>
                    {OFFER_TYPES.map((o) => (
                      <Chip key={o} label={o} on={v.offer_type === o} onPress={() => set('offer_type', o)} />
                    ))}
                  </ScrollView>
                ) : null}
              </View>
            ))}
          </View>
        ))}
        <View style={{ gap: 14, backgroundColor: c.surface, borderColor: c.line, borderWidth: 1, borderRadius: radius, padding: 16 }}>
          <T style={{ fontFamily: fonts.display, fontSize: 17, color: c.accentDeep }}>التاريخ والحالة</T>
          <Field
            testID="field-listed_on"
            label="اليوم والتاريخ (ميلادي)"
            placeholder="اتركه فارغاً لتاريخ اليوم"
            value={v.listed_on ?? ''}
            onChangeText={(t) => set('listed_on', t)}
            keyboardType="numbers-and-punctuation"
          />
          {dateHint ? <T style={{ color: c.muted, fontSize: 13 }}>{dateHint}</T> : null}
          <T style={{ fontFamily: fonts.medium, fontSize: 15 }}>الحالة</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            {STATUSES.map((s) => (
              <Chip key={s.value} testID={`status-${s.value}`} label={s.label} on={(v.status ?? 'available') === s.value} onPress={() => set('status', s.value)} />
            ))}
          </View>
        </View>
        <Button testID="save" label="حفظ" busy={busy} onPress={() => onSave(v)} />
        {onDelete ? <Button testID="delete" label="حذف العرض" kind="danger" onPress={onDelete} /> : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}
