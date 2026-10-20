import * as Haptics from 'expo-haptics';
import { Image } from 'expo-image';
import { useRef, useState } from 'react';
import { ActivityIndicator, Platform, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';

import { imageSource, type EditSource, type JobKind, type WorkflowOption } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useJobs } from '../lib/jobsStore';
import { pickPicture, uploadPicture } from '../lib/pictures';
import { colors, maxFontScale, radius } from '../lib/theme';
import { Icon } from './Icon';

/** The picture an edit starts from: one picked on this iPhone, or an image in the history. */
export type Picture =
  | { from: 'library'; uri: string; /** Null while it uploads. */ upload: string | null }
  | { from: 'history'; job: string; index: number; preview: string };

function sourceOf(picture: Picture | null): EditSource | null {
  if (!picture) return null;
  if (picture.from === 'history') return { job: picture.job, index: picture.index };
  return picture.upload ? { upload: picture.upload } : null;
}

/** Words a LoRA was trained on, offered while it is on: Anything2Real expects this opening. */
const optionLeads: Record<string, string> = { realism: 'change the picture 1 to realistic photograph, ' };

type Props = {
  /** What the Mac takes now. */
  kind: JobKind;
  value: string;
  onChange: (text: string) => void;
  maxLength?: number;
  disabled: boolean;
  bottomInset: number;
  /** The workflow's switches, and how they are set. */
  options: WorkflowOption[];
  chosen: Record<string, boolean>;
  onToggle: (name: string) => void;
  picture: Picture | null;
  onPicture: (picture: Picture | null) => void;
};

/** The prompt box, pinned to the bottom like a chat composer. Everything else is fixed by the workflow. */
export function Composer({ kind, value, onChange, maxLength, disabled, bottomInset, options, chosen, onToggle, picture, onPicture }: Props) {
  const { connection } = useConnection();
  const { generate } = useJobs();
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const picking = useRef(0);
  const editing = kind === 'edit';
  const source = sourceOf(picture);
  const canSend = value.trim().length > 0 && !sending && !disabled && (!editing || source != null);

  const choosePicture = async () => {
    if (!connection) return;
    setError(null);
    const picked = await pickPicture().catch((e: unknown) => {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    });
    if (!picked) return;
    // A newer pick replaces this one even while it uploads.
    const attempt = ++picking.current;
    onPicture({ from: 'library', uri: picked.uri, upload: null });
    try {
      const upload = await uploadPicture(connection, picked.uri);
      if (attempt === picking.current) onPicture({ from: 'library', uri: picked.uri, upload });
    } catch (e) {
      if (attempt !== picking.current) return;
      onPicture(null);
      setError(`Couldn't send the picture: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const send = async () => {
    if (!canSend) return;
    setSending(true);
    setError(null);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      const switches = Object.fromEntries(options.map((o) => [o.name, chosen[o.name] ?? o.on]));
      await generate({ prompt: value.trim(), kind, source: editing ? (source ?? undefined) : undefined, options: switches });
      // The picture stays, so the next edit can try other words on it.
      onChange('');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
    } finally {
      setSending(false);
    }
  };

  const lead = options
    .filter((o) => chosen[o.name] ?? o.on)
    .map((o) => optionLeads[o.name.toLowerCase()])
    .find((text) => text && !value.toLowerCase().includes(text.trim().replace(/,$/, '')));
  const showTools = (editing || options.length > 0) && !disabled;

  return (
    <View style={[styles.wrap, { paddingBottom: Math.max(bottomInset, 10) }]}>
      {error ? (
        <Text style={styles.error} accessibilityRole="alert" maxFontSizeMultiplier={maxFontScale}>
          {error}
        </Text>
      ) : null}

      {showTools ? (
        <View style={styles.tools}>
          {editing && picture && connection ? (
            <View>
              <Pressable onPress={choosePicture} accessibilityRole="imagebutton" accessibilityLabel="Picture to edit. Choose another." hitSlop={4}>
                <Image
                  source={picture.from === 'library' ? { uri: picture.uri } : imageSource(connection, picture.preview)}
                  style={styles.thumb}
                  contentFit="cover"
                />
                {picture.from === 'library' && !picture.upload ? (
                  <View style={styles.thumbBusy}>
                    <ActivityIndicator color={colors.text} />
                  </View>
                ) : null}
              </Pressable>
              <Pressable
                onPress={() => {
                  picking.current++;
                  onPicture(null);
                }}
                accessibilityRole="button"
                accessibilityLabel="Remove the picture"
                hitSlop={8}
                style={styles.thumbClear}
              >
                <Icon name="xmark.circle.fill" size={18} color={colors.text} fallback="✕" />
              </Pressable>
            </View>
          ) : null}
          {options.map((option) => {
            const on = chosen[option.name] ?? option.on;
            return (
              <Pressable
                key={option.name}
                onPress={() => onToggle(option.name)}
                accessibilityRole="switch"
                accessibilityLabel={option.name}
                accessibilityState={{ checked: on }}
                style={[styles.chip, on && styles.chipOn]}
                hitSlop={4}
              >
                <Text style={[styles.chipText, on && styles.chipTextOn]} maxFontSizeMultiplier={1.3}>
                  {option.name}
                </Text>
              </Pressable>
            );
          })}
        </View>
      ) : null}
      {lead && !disabled ? (
        <Pressable onPress={() => onChange(lead + value.trimStart())} accessibilityRole="button" style={styles.lead} hitSlop={4}>
          <Text style={styles.leadText} maxFontSizeMultiplier={maxFontScale}>
            Starts best with “{lead.trim().replace(/,$/, '')}”. <Text style={styles.leadAction}>Insert</Text>
          </Text>
        </Pressable>
      ) : null}

      <View style={styles.box}>
        {editing && !picture ? (
          <Pressable
            onPress={choosePicture}
            disabled={disabled}
            accessibilityRole="button"
            accessibilityLabel="Choose a picture to edit"
            style={[styles.pick, disabled && styles.sendOff]}
            hitSlop={6}
          >
            <Icon name="photo.badge.plus" size={20} color={colors.accent} fallback="＋" />
          </Pressable>
        ) : null}
        <TextInput
          style={styles.input}
          value={value}
          onChangeText={onChange}
          placeholder={editing ? (picture ? 'Describe the edit…' : 'Choose a picture, then describe the edit…') : 'Describe the image…'}
          placeholderTextColor={colors.faint}
          multiline
          maxLength={maxLength}
          editable={!disabled}
          accessibilityLabel={editing ? 'Edit instruction' : 'Prompt'}
          maxFontSizeMultiplier={maxFontScale}
          keyboardAppearance="dark"
          // Return inserts a new line; the arrow sends. On a hardware keyboard Cmd+Return also sends.
          onKeyPress={(e) => {
            const key = e.nativeEvent as unknown as { key: string; metaKey?: boolean; ctrlKey?: boolean };
            if (Platform.OS === 'web' && key.key === 'Enter' && (key.metaKey || key.ctrlKey)) void send();
          }}
        />
        <Pressable
          onPress={send}
          disabled={!canSend}
          accessibilityRole="button"
          accessibilityLabel={editing ? 'Edit' : 'Generate'}
          accessibilityState={{ disabled: !canSend, busy: sending }}
          style={[styles.send, !canSend && styles.sendOff]}
          hitSlop={6}
        >
          {sending ? (
            <ActivityIndicator color={colors.accentText} />
          ) : (
            <Icon name="arrow.up" size={18} weight="bold" color={colors.accentText} fallback="↑" />
          )}
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    paddingHorizontal: 12,
    paddingTop: 8,
    backgroundColor: colors.background,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
  },
  error: { color: colors.danger, fontSize: 13, lineHeight: 18, marginBottom: 6 },
  tools: { flexDirection: 'row', alignItems: 'center', flexWrap: 'wrap', gap: 8, marginBottom: 8 },
  thumb: { width: 52, height: 52, borderRadius: 10, backgroundColor: colors.card },
  thumbBusy: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: 'rgba(0,0,0,0.45)',
  },
  thumbClear: { position: 'absolute', top: -7, right: -7, backgroundColor: colors.background, borderRadius: 10 },
  chip: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 15,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    backgroundColor: colors.field,
  },
  chipOn: { backgroundColor: colors.accent, borderColor: colors.accent },
  chipText: { color: colors.muted, fontSize: 13, fontWeight: '600' },
  chipTextOn: { color: colors.accentText },
  lead: { marginBottom: 8 },
  leadText: { color: colors.muted, fontSize: 13, lineHeight: 18 },
  leadAction: { color: colors.accent, fontWeight: '600' },
  box: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    gap: 8,
    backgroundColor: colors.field,
    borderRadius: radius.bubble + 4,
    paddingLeft: 14,
    paddingRight: 6,
    paddingVertical: 6,
  },
  pick: { width: 30, height: 34, marginLeft: -6, alignItems: 'center', justifyContent: 'center' },
  input: { flex: 1, color: colors.text, fontSize: 16, lineHeight: 21, maxHeight: 140, paddingTop: 7, paddingBottom: 7 },
  send: { width: 34, height: 34, borderRadius: 17, backgroundColor: colors.accent, alignItems: 'center', justifyContent: 'center' },
  sendOff: { opacity: 0.35 },
});
