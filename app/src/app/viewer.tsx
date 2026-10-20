import * as Haptics from 'expo-haptics';
import { Image } from 'expo-image';
import { router, useLocalSearchParams } from 'expo-router';
import type { SFSymbol } from 'expo-symbols';
import { useState } from 'react';
import { AccessibilityInfo, ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Icon } from '../components/Icon';
import { imageSource } from '../lib/api';
import { useConnection } from '../lib/connection';
import { showMessage } from '../lib/dialogs';
import { useJobs } from '../lib/jobsStore';
import { saveToPhotos, shareImage } from '../lib/save';
import { colors } from '../lib/theme';

function Button({ icon, fallback, label, onPress, busy }: { icon: SFSymbol; fallback: string; label: string; onPress: () => void; busy?: boolean }) {
  return (
    <Pressable onPress={onPress} disabled={busy} accessibilityRole="button" accessibilityLabel={label} style={styles.button} hitSlop={8}>
      {busy ? <ActivityIndicator color={colors.text} /> : <Icon name={icon} size={22} fallback={fallback} />}
    </Pressable>
  );
}

/** One image, full size: pinch to zoom, save or share. */
export default function Viewer() {
  const { id, n } = useLocalSearchParams<{ id: string; n?: string }>();
  const { connection } = useConnection();
  const { jobs, aspectRatio } = useJobs();
  const insets = useSafeAreaInsets();
  const { width } = useWindowDimensions();
  const [busy, setBusy] = useState<null | 'save' | 'share'>(null);
  const job = jobs.find((j) => j.id === id);
  const image = job?.images?.[Number(n ?? 0)];

  const close = () => (router.canGoBack() ? router.back() : router.replace('/'));

  async function run(kind: 'save' | 'share', work: () => Promise<void>, done?: string) {
    setBusy(kind);
    try {
      await work();
      if (done) {
        AccessibilityInfo.announceForAccessibility(done);
        void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      }
    } catch (e) {
      showMessage("That didn't work", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <View style={styles.screen}>
      {connection && job && image ? (
        <ScrollView
          style={styles.screen}
          contentContainerStyle={styles.zoomed}
          maximumZoomScale={5}
          minimumZoomScale={1}
          centerContent
          bouncesZoom
          showsHorizontalScrollIndicator={false}
          showsVerticalScrollIndicator={false}
        >
          <Image
            source={imageSource(connection, image.url)}
            placeholder={imageSource(connection, image.preview)}
            style={{ width, aspectRatio }}
            contentFit="contain"
            cachePolicy="disk"
            accessibilityLabel={`Image for: ${job.prompt}`}
          />
        </ScrollView>
      ) : (
        <View style={[styles.screen, styles.zoomed]}>
          <Text style={styles.missing}>This image is no longer in the history.</Text>
        </View>
      )}

      <View style={[styles.top, { paddingTop: insets.top + 6 }]}>
        <Button icon="xmark" fallback="✕" label="Close" onPress={close} />
      </View>

      {connection && job && image ? (
        <View style={[styles.bottom, { paddingBottom: insets.bottom + 8 }]}>
          <Button
            icon="square.and.arrow.down"
            fallback="⤓"
            label="Save to Photos"
            busy={busy === 'save'}
            onPress={() => run('save', () => saveToPhotos(connection, job.id, image), 'Saved to Photos')}
          />
          <Button
            icon="square.and.arrow.up"
            fallback="⇪"
            label="Share"
            busy={busy === 'share'}
            onPress={() => run('share', () => shareImage(connection, job.id, image))}
          />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: '#000' },
  zoomed: { flexGrow: 1, alignItems: 'center', justifyContent: 'center' },
  missing: { color: colors.muted, fontSize: 15 },
  top: { position: 'absolute', left: 0, right: 0, top: 0, flexDirection: 'row', paddingHorizontal: 12 },
  bottom: { position: 'absolute', left: 0, right: 0, bottom: 0, flexDirection: 'row', justifyContent: 'space-around', paddingTop: 8 },
  button: { width: 48, height: 48, borderRadius: 24, alignItems: 'center', justifyContent: 'center', backgroundColor: 'rgba(40,40,44,0.8)' },
});
