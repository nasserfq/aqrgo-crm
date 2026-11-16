// The web app's lavender design system (crates/common/static/app.css), for the phone.
import { useColorScheme } from 'react-native';

const light = {
  bg: '#f7f5fc',
  surface: '#ffffff',
  ink: '#1b1530',
  muted: '#6d6585',
  line: '#e4def3',
  lineStrong: '#cfc5ea',
  accent: '#6c4fd8',
  accentDeep: '#3d2a8a',
  accentInk: '#ffffff',
  accentSoft: '#efeafd',
  primary: '#1b1530',
  primaryInk: '#ffffff',
  danger: '#b3261e',
  dangerSoft: '#fdecea',
  warn: '#8a5a00',
  warnSoft: '#fdf3dd',
  sold: '#5b5770',
  soldSoft: '#eeedf2',
};

export type Colors = typeof light;

const dark: Colors = {
  bg: '#110d1f',
  surface: '#1a1530',
  ink: '#ece8f8',
  muted: '#a59dc0',
  line: '#2c2547',
  lineStrong: '#3d345f',
  accent: '#b49cf5',
  accentDeep: '#d5c8fb',
  accentInk: '#110d1f',
  accentSoft: '#2a2150',
  primary: '#b49cf5',
  primaryInk: '#110d1f',
  danger: '#f2a49d',
  dangerSoft: '#3a1a1c',
  warn: '#f0c46e',
  warnSoft: '#3a2c10',
  sold: '#b9b3cc',
  soldSoft: '#2a263a',
};

export function useColors(): Colors {
  return useColorScheme() === 'dark' ? dark : light;
}

export const fonts = {
  body: 'IBMPlexSansArabic_400Regular',
  medium: 'IBMPlexSansArabic_500Medium',
  bold: 'IBMPlexSansArabic_700Bold',
  display: 'Alexandria_800ExtraBold',
};

export const radius = 4;

/** Badge colours per status, like the web table. */
export function statusColors(c: Colors, status: string) {
  if (status === 'sold') return { bg: c.soldSoft, fg: c.sold };
  if (status === 'reserved') return { bg: c.warnSoft, fg: c.warn };
  return { bg: c.accentSoft, fg: c.accentDeep };
}
