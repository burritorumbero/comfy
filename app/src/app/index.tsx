import { Redirect, router } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { FlatList, KeyboardAvoidingView, Platform, Pressable, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Banner } from '../components/Banner';
import { Composer, type Picture } from '../components/Composer';
import { Icon } from '../components/Icon';
import { JobCard } from '../components/JobCard';
import { ModeBar } from '../components/ModeBar';
import { ServicesCard } from '../components/ServicesCard';
import type { Job, Mode } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useJobs } from '../lib/jobsStore';
import { headline, imageKindOf, statusOf } from '../lib/services';
import { useRequestSwitch, useServices } from '../lib/servicesStore';
import { colors, maxFontScale } from '../lib/theme';

export default function Home() {
  const { ready, connection } = useConnection();
  if (!ready) return <View style={styles.screen} />;
  if (!connection) return <Redirect href="/connect" />;
  return <Feed />;
}

/** The history, newest at the bottom next to the composer, as in a chat. */
function Feed() {
  const insets = useSafeAreaInsets();
  const { rejected } = useConnection();
  const { jobs, info, problem, refresh, loadOlder } = useJobs();
  const { services } = useServices();
  const requestSwitch = useRequestSwitch();
  const [draft, setDraft] = useState('');
  const [picture, setPicture] = useState<Picture | null>(null);
  const [chosen, setChosen] = useState<Record<string, boolean>>({});
  const [notice, setNotice] = useState<string | null>(null);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(noticeTimer.current), []);

  const showNotice = useCallback((text: string) => {
    setNotice(text);
    clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(null), 1800);
  }, []);

  // The Mac takes one kind of image job at a time; while it takes none, the composer keeps the last.
  const takes = imageKindOf(services);
  const [lastKind, setLastKind] = useState(takes ?? 'generate');
  if (takes && takes !== lastKind) setLastKind(takes);
  const kind = takes ?? lastKind;
  const workflows = info?.workflows;
  const canEdit = services != null && services.modes.includes('edit') && workflows?.edit != null;
  const options = (kind === 'edit' ? workflows?.edit?.options : workflows?.generate.options) ?? [];

  // "Edit this image" switches the Mac to Edit first when it is set up for something else.
  const editImage = useCallback(
    async (job: Job, index: number) => {
      const preview = job.images?.[index]?.preview;
      if (!preview) return;
      if (takes !== 'edit' && !(await requestSwitch('edit'))) return;
      setPicture({ from: 'history', job: job.id, index, preview });
    },
    [takes, requestSwitch],
  );
  const onSwitched = useCallback((to: Mode) => {
    if (to !== 'edit') setPicture(null);
  }, []);

  const renderItem = useCallback(
    ({ item }: { item: Job }) => (
      <JobCard job={item} onUsePrompt={setDraft} onNotice={showNotice} onEdit={canEdit ? editImage : undefined} />
    ),
    [showNotice, canEdit, editImage],
  );

  // With switching, the status says what the Mac is set up for; otherwise only whether ComfyUI answers.
  const status = services ? statusOf(services) : null;
  const statusLine = services ? headline(services) : info ? (info.comfy ? 'ComfyUI ready' : 'ComfyUI offline') : 'Connecting…';
  const statusColor = status
    ? { generate: colors.ok, edit: colors.ok, llm: colors.accent, switching: colors.accent, off: colors.danger }[status]
    : info
      ? info.comfy
        ? colors.ok
        : colors.danger
      : colors.faint;

  return (
    <KeyboardAvoidingView style={styles.screen} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <View style={[styles.header, { paddingTop: insets.top + 6 }]}>
        <View style={styles.titleBlock}>
          <Text style={styles.title} numberOfLines={1} maxFontSizeMultiplier={1.3}>
            {info?.title ?? 'Prompt'}
          </Text>
          <View style={styles.statusRow} accessible accessibilityLabel={statusLine}>
            <View style={[styles.dot, { backgroundColor: statusColor }]} />
            <Text style={styles.subtitle} maxFontSizeMultiplier={1.3}>
              {statusLine}
            </Text>
          </View>
        </View>
        <Pressable onPress={() => router.push('/settings')} accessibilityRole="button" accessibilityLabel="Settings" hitSlop={10} style={styles.gear}>
          <Icon name="gearshape" size={22} color={colors.muted} fallback="⚙" />
        </Pressable>
      </View>

      {services && !rejected ? <ModeBar services={services} onSwitched={onSwitched} /> : null}

      {rejected ? (
        <Banner
          tone="danger"
          text="Your Mac no longer accepts the saved password. It was probably changed there."
          action="Update password"
          onAction={() => router.push('/connect')}
        />
      ) : problem ? (
        <Banner tone="warn" text={problem} action="Try again" onAction={refresh} />
      ) : services ? (
        <ServicesCard services={services} />
      ) : info && !info.comfy ? (
        <Banner tone="warn" text="ComfyUI isn't answering on your Mac. New prompts wait until it's back." />
      ) : null}

      {jobs.length === 0 ? (
        <View style={styles.empty}>
          <Text style={styles.emptyTitle} maxFontSizeMultiplier={maxFontScale}>
            Your images will appear here
          </Text>
          <Text style={styles.emptyText} maxFontSizeMultiplier={maxFontScale}>
            {kind === 'edit'
              ? 'Choose a picture below and say what to change, like “make it night” or “remove the people”.'
              : 'Describe what you want below. Everything else (model, size, steps) is fixed by the workflow on your Mac'}
            {kind === 'edit' ? '' : info?.seed === 'random' ? ', with a new seed each time.' : '.'}
          </Text>
        </View>
      ) : (
        <FlatList
          inverted
          data={jobs}
          keyExtractor={(job) => job.id}
          renderItem={renderItem}
          onEndReached={loadOlder}
          onEndReachedThreshold={0.6}
          contentContainerStyle={styles.list}
          ItemSeparatorComponent={Separator}
          keyboardDismissMode="interactive"
          keyboardShouldPersistTaps="handled"
          initialNumToRender={4}
          windowSize={7}
        />
      )}

      {notice ? (
        <View style={styles.notice} pointerEvents="none">
          <Text style={styles.noticeText}>{notice}</Text>
        </View>
      ) : null}

      <Composer
        kind={kind}
        value={draft}
        onChange={setDraft}
        maxLength={info?.maxPromptChars}
        disabled={rejected || takes == null}
        bottomInset={insets.bottom}
        options={options}
        chosen={chosen}
        onToggle={(name) =>
          setChosen((now) => ({ ...now, [name]: !(now[name] ?? options.find((o) => o.name === name)?.on ?? false) }))
        }
        picture={picture}
        onPicture={setPicture}
      />
    </KeyboardAvoidingView>
  );
}

function Separator() {
  return <View style={styles.separator} />;
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.background },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingBottom: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.border,
  },
  titleBlock: { flex: 1, gap: 2 },
  title: { color: colors.text, fontSize: 19, fontWeight: '700' },
  statusRow: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  subtitle: { color: colors.muted, fontSize: 13 },
  gear: { padding: 6 },
  list: { paddingHorizontal: 12, paddingVertical: 14 },
  separator: { height: 22 },
  empty: { flex: 1, justifyContent: 'center', paddingHorizontal: 32, gap: 8 },
  emptyTitle: { color: colors.text, fontSize: 17, fontWeight: '600', textAlign: 'center' },
  emptyText: { color: colors.muted, fontSize: 14, lineHeight: 20, textAlign: 'center' },
  notice: {
    position: 'absolute',
    alignSelf: 'center',
    top: '45%',
    backgroundColor: colors.bubble,
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderRadius: 20,
  },
  noticeText: { color: colors.text, fontSize: 15, fontWeight: '600' },
});
