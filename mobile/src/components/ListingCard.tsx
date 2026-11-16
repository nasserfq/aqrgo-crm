// One row of the table as a phone card, like the web's phone view.
import { Pressable, View } from 'react-native';

import type { Listing } from '../lib/api';
import { fonts, radius, statusColors, useColors } from '../lib/theme';
import { T } from './ui';

const DETAILS: { key: keyof Listing; label: string }[] = [
  { key: 'plan_no', label: 'المخطط' },
  { key: 'plot_no', label: 'القطعة' },
  { key: 'area', label: 'المساحة' },
  { key: 'price', label: 'السعر' },
];

export function StatusPill({ status, label }: { status: string; label: string }) {
  const c = useColors();
  const s = statusColors(c, status);
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, backgroundColor: s.bg, paddingHorizontal: 10, paddingVertical: 2, borderRadius: radius }}>
      <View style={{ width: 6, height: 6, borderRadius: 1, backgroundColor: s.fg }} />
      <T style={{ color: s.fg, fontSize: 13, fontFamily: fonts.medium }}>{label}</T>
    </View>
  );
}

export function ListingCard({ listing, onPress }: { listing: Listing; onPress: () => void }) {
  const c = useColors();
  const details = DETAILS.filter((d) => String(listing[d.key] ?? '').trim());
  return (
    <Pressable
      testID={`listing-${listing.id}`}
      accessibilityRole="button"
      onPress={onPress}
      style={({ pressed }) => ({
        backgroundColor: c.surface,
        borderColor: c.line,
        borderWidth: 1,
        borderRadius: radius,
        padding: 16,
        gap: 10,
        opacity: pressed ? 0.85 : 1,
      })}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
        <T style={{ fontFamily: fonts.display, fontSize: 20, color: c.accentDeep, minWidth: 28 }}>{listing.number}</T>
        <View style={{ flex: 1 }}>
          <T style={{ fontFamily: fonts.bold, fontSize: 17 }}>{listing.offer_type || 'عرض'}</T>
          {listing.location ? <T style={{ color: c.muted }}>{listing.location}</T> : null}
        </View>
        <StatusPill status={listing.status} label={listing.status_label} />
      </View>
      {details.length ? (
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 16 }}>
          {details.map((d) => (
            <View key={d.key}>
              <T style={{ fontSize: 12, color: c.muted }}>{d.label}</T>
              <T style={{ fontFamily: fonts.medium }}>{String(listing[d.key])}</T>
            </View>
          ))}
        </View>
      ) : null}
      <T style={{ fontSize: 13, color: c.muted }}>
        {listing.date_hijri} · {listing.date_gregorian}
      </T>
    </Pressable>
  );
}
