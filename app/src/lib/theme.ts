// Dark, dense and calm: the phone is a control surface for the Mac, not a gallery app.
export const colors = {
  background: '#000000',
  surface: '#111113',
  card: '#18181b',
  bubble: '#2a2a2e',
  border: '#2c2c31',
  field: '#1f1f23',
  text: '#f2f2f4',
  muted: '#8e8e93',
  faint: '#5c5c62',
  accent: '#f5b82e',
  accentText: '#1a1300',
  danger: '#ff6b5e',
  ok: '#5fd38a',
} as const;

export const radius = { card: 16, bubble: 18, control: 12 } as const;

// Text grows with the user's Dynamic Type setting, up to where dense rows still fit.
export const maxFontScale = 1.6;
