import * as Clipboard from 'expo-clipboard';
import * as Haptics from 'expo-haptics';
import { Image } from 'expo-image';
import { router } from 'expo-router';
import type { SFSymbol } from 'expo-symbols';
import { memo, useState } from 'react';
import { AccessibilityInfo, ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native';

import { imageSource, type Job, type JobImage } from '../lib/api';
import { useConnection } from '../lib/connection';
import { confirmDestructive, showMessage } from '../lib/dialogs';
import { isActive, statusText } from '../lib/jobs';
import { useJobs } from '../lib/jobsStore';
import { saveToPhotos, shareImage } from '../lib/save';
import { colors, maxFontScale, radius } from '../lib/theme';
import { useNow } from '../lib/useNow';
import { Icon } from './Icon';

type ActionProps = { icon: SFSymbol; fallback: string; label: string; onPress: () => void; busy?: boolean; tint?: string };

function Action({ icon, fallback, label, onPress, busy, tint = colors.text }: ActionProps) {
  return (
    <Pressable
      onPress={onPress}
      disabled={busy}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ busy }}
      style={({ pressed }) => [styles.action, pressed && styles.pressed]}
      hitSlop={4}
    >
      {busy ? <ActivityIndicator color={tint} /> : <Icon name={icon} size={19} color={tint} fallback={fallback} />}
    </Pressable>
  );
}

type Props = {
  job: Job;
  onUsePrompt: (prompt: string) => void;
  onNotice: (text: string) => void;
  /** Start an edit of one of this job's images. Missing when the server can't edit. */
  onEdit?: (job: Job, index: number) => void;
};

/** One prompt and what came of it. */
export const JobCard = memo(function JobCard({ job, onUsePrompt, onNotice, onEdit }: Props) {
  const { connection } = useConnection();
  const { runningSince, aspectRatio, setAspectRatio, cancel, remove } = useJobs();
  const active = isActive(job);
  const now = useNow(job.state === 'running');
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState<null | 'save' | 'share' | 'cancel' | 'remove'>(null);
  const failed = job.state === 'error' || job.state === 'lost';
  const images = job.state === 'done' ? (job.images ?? []) : [];
  const first: JobImage | undefined = images[0];
  const edit = job.kind === 'edit';

  async function run(kind: NonNullable<typeof busy>, work: () => Promise<void>, done?: string) {
    setBusy(kind);
    try {
      await work();
      if (done) {
        onNotice(done);
        AccessibilityInfo.announceForAccessibility(done);
        void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      }
    } catch (e) {
      showMessage("That didn't work", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const askRemove = async () => {
    const ok = await confirmDestructive(
      'Remove from history?',
      "It disappears from every device. The image file stays in ComfyUI's output folder on your Mac.",
      'Remove',
    );
    if (ok) void run('remove', () => remove(job.id));
  };

  if (!connection) return null;
  const progress = job.state === 'running' && job.progress && job.progress.max > 1 ? job.progress : null;

  return (
    <View style={styles.card}>
      <View style={styles.bubbleRow}>
        {job.source ? (
          <Image
            source={imageSource(connection, job.source)}
            style={styles.source}
            contentFit="cover"
            cachePolicy="disk"
            recyclingKey={job.source}
            accessibilityLabel="The picture this edit started from"
            accessible
          />
        ) : null}
        <Pressable
          onPress={() => setExpanded((e) => !e)}
          accessibilityRole="button"
          accessibilityHint={expanded ? 'Shows less of the prompt' : 'Shows the whole prompt'}
          style={styles.bubbleWrap}
        >
          <Text style={styles.bubble} numberOfLines={expanded ? undefined : 4} maxFontSizeMultiplier={maxFontScale}>
            {job.prompt || '(no prompt saved)'}
          </Text>
        </Pressable>
      </View>

      {images.map((image, n) => (
        <Pressable
          key={image.url}
          onPress={() => router.push({ pathname: '/viewer', params: { id: job.id, n: String(n) } })}
          accessibilityRole="imagebutton"
          accessibilityLabel={`${edit ? 'Edit' : 'Image'} for: ${job.prompt}. Opens full screen.`}
        >
          <Image
            source={imageSource(connection, image.preview)}
            style={[styles.image, { aspectRatio }]}
            contentFit="cover"
            cachePolicy="disk"
            recyclingKey={image.url}
            onLoad={(e) => {
              const { width, height } = e.source;
              if (width && height) setAspectRatio(width / height);
            }}
          />
        </Pressable>
      ))}

      {active ? (
        <View style={[styles.waiting, { aspectRatio }]}>
          <Text style={styles.waitingText} maxFontSizeMultiplier={maxFontScale}>
            {job.state === 'queued' ? 'Waiting for ComfyUI' : edit ? 'Editing' : 'Generating'}
          </Text>
          {progress ? (
            <View
              style={styles.track}
              accessibilityRole="progressbar"
              accessibilityValue={{ min: 0, max: progress.max, now: progress.value }}
            >
              <View style={[styles.fill, { width: `${Math.round((100 * progress.value) / progress.max)}%` }]} />
            </View>
          ) : null}
        </View>
      ) : null}

      <Text style={[styles.status, failed && styles.statusFailed]} maxFontSizeMultiplier={maxFontScale}>
        {statusText(job, now, runningSince.get(job.id))}
      </Text>

      <View style={styles.actions}>
        {first ? (
          <>
            <Action
              icon="square.and.arrow.down"
              fallback="⤓"
              label="Save to Photos"
              busy={busy === 'save'}
              onPress={() => run('save', () => saveToPhotos(connection, job.id, first), 'Saved to Photos')}
            />
            <Action
              icon="square.and.arrow.up"
              fallback="⇪"
              label="Share"
              busy={busy === 'share'}
              onPress={() => run('share', () => shareImage(connection, job.id, first))}
            />
            {onEdit ? (
              <Action icon="wand.and.stars" fallback="✎" label="Edit this image" onPress={() => onEdit(job, 0)} />
            ) : null}
          </>
        ) : null}
        <Action icon="arrow.uturn.backward" fallback="↩" label="Use this prompt again" onPress={() => onUsePrompt(job.prompt)} />
        <Action
          icon="doc.on.doc"
          fallback="⧉"
          label="Copy prompt"
          onPress={() => {
            void Clipboard.setStringAsync(job.prompt);
            onNotice('Prompt copied');
          }}
        />
        <View style={styles.spacer} />
        {active ? (
          <Action
            icon="xmark.circle"
            fallback="✕"
            label="Cancel"
            tint={colors.muted}
            busy={busy === 'cancel'}
            onPress={() => run('cancel', () => cancel(job.id))}
          />
        ) : (
          <Action icon="trash" fallback="🗑" label="Remove from history" tint={colors.muted} busy={busy === 'remove'} onPress={askRemove} />
        )}
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  card: { gap: 8 },
  bubbleRow: { alignSelf: 'flex-end', maxWidth: '88%', flexDirection: 'row', alignItems: 'flex-end', gap: 8 },
  bubbleWrap: { flexShrink: 1 },
  source: { width: 44, height: 44, borderRadius: 10, backgroundColor: colors.card },
  bubble: {
    backgroundColor: colors.bubble,
    color: colors.text,
    fontSize: 15,
    lineHeight: 20,
    paddingHorizontal: 14,
    paddingVertical: 9,
    borderRadius: radius.bubble,
    overflow: 'hidden',
  },
  image: { width: '100%', borderRadius: radius.card, backgroundColor: colors.card },
  waiting: {
    width: '100%',
    maxHeight: 420,
    borderRadius: radius.card,
    backgroundColor: colors.card,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 12,
  },
  waitingText: { color: colors.muted, fontSize: 15 },
  track: { width: '60%', height: 4, borderRadius: 2, backgroundColor: colors.border, overflow: 'hidden' },
  fill: { height: '100%', backgroundColor: colors.accent },
  status: { color: colors.muted, fontSize: 13, lineHeight: 17, paddingHorizontal: 2 },
  statusFailed: { color: colors.danger },
  actions: { flexDirection: 'row', alignItems: 'center', gap: 2 },
  action: { minWidth: 44, height: 40, alignItems: 'center', justifyContent: 'center', borderRadius: 10 },
  pressed: { backgroundColor: colors.card },
  spacer: { flex: 1 },
});
