import Constants from "expo-constants";
import { router } from "expo-router";
import type { ReactNode } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { hostOf } from "../lib/address";
import { useConnection } from "../lib/connection";
import { confirmDestructive, showMessage } from "../lib/dialogs";
import { useJobs } from "../lib/jobsStore";
import { usePreferences } from "../lib/preferences";
import { clearImages } from "../lib/save";
import {
  imagesSummary,
  llmStoppedHint,
  llmSummary,
  modeLabel,
  selectedMode,
  switchingText,
} from "../lib/services";
import { useRequestSwitch, useServices } from "../lib/servicesStore";
import { colors, maxFontScale, radius } from "../lib/theme";
import { useNow } from "../lib/useNow";

function Section({
  title,
  footer,
  children,
}: {
  title: string;
  footer?: string;
  children: ReactNode;
}) {
  return (
    <View style={styles.section}>
      <Text
        style={styles.sectionTitle}
        maxFontSizeMultiplier={maxFontScale}
        accessibilityRole="header"
      >
        {title.toUpperCase()}
      </Text>
      <View style={styles.group}>{children}</View>
      {footer ? (
        <Text style={styles.footer} maxFontSizeMultiplier={maxFontScale}>
          {footer}
        </Text>
      ) : null}
    </View>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <View
      style={styles.row}
      accessible
      accessibilityLabel={`${label}: ${value}`}
    >
      <Text style={styles.rowLabel} maxFontSizeMultiplier={maxFontScale}>
        {label}
      </Text>
      <Text
        style={styles.rowValue}
        numberOfLines={2}
        selectable
        maxFontSizeMultiplier={maxFontScale}
      >
        {value}
      </Text>
    </View>
  );
}

function ButtonRow({
  label,
  onPress,
  destructive,
}: {
  label: string;
  onPress: () => void;
  destructive?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
    >
      <Text
        style={[styles.button, destructive && styles.destructive]}
        maxFontSizeMultiplier={maxFontScale}
      >
        {label}
      </Text>
    </Pressable>
  );
}

export default function Settings() {
  const insets = useSafeAreaInsets();
  const { connection, forget } = useConnection();
  const { info, problem } = useJobs();
  const { services } = useServices();
  const requestSwitch = useRequestSwitch();
  const now = useNow(services?.switching != null);
  const { hideInSwitcher, setHideInSwitcher } = usePreferences();

  const forgetServer = async () => {
    const ok = await confirmDestructive(
      "Forget this server?",
      "The address, password, cached history and downloaded images are removed from this iPhone. Everything on your Mac stays.",
      "Forget",
    );
    if (!ok) return;
    await forget();
    router.dismissAll();
    router.replace("/connect");
  };

  const clearDownloads = async () => {
    await clearImages();
    showMessage(
      "Images cleared",
      "Downloaded and cached images were removed from this iPhone. They load again from your Mac when shown.",
    );
  };

  const comfy = problem
    ? "Can't reach your Mac"
    : info
      ? info.comfy
        ? "Answering"
        : "Not answering"
      : "Checking…";

  return (
    <ScrollView
      style={styles.screen}
      contentContainerStyle={{ paddingBottom: insets.bottom + 24 }}
    >
      <Section title="Server">
        <Row
          label="Address"
          value={connection ? hostOf(connection.address) : "—"}
        />
        <Row label="Workflow" value={info?.title ?? "—"} />
        {services ? null : <Row label="ComfyUI" value={comfy} />}
        <Row label="Server version" value={info?.version ?? "—"} />
        <ButtonRow
          label="Change address or password"
          onPress={() => router.push("/connect")}
        />
      </Section>

      {services ? (
        <Section
          title="Your Mac"
          footer={[
            llmStoppedHint(services),
            "Generating, editing and the LLM each need a large model, so your Mac holds one at a time. Switching frees the current model before the next loads; both apps stay open. If Hermes loads the LLM by itself, Prompt frees ComfyUI and pauses images.",
          ]
            .filter(Boolean)
            .join("\n\n")}
        >
          <Row label="Mode" value={modeLabel[selectedMode(services)]} />
          <Row label="Images (ComfyUI)" value={imagesSummary(services)} />
          <Row label="LLM (MLX-Serve)" value={llmSummary(services)} />
          {services.error && !services.switching ? (
            <Row label="Last switch" value={services.error} />
          ) : null}
          {services.notice && !services.switching ? (
            <Row label="Last change" value={services.notice} />
          ) : null}
          {services.switching ? (
            <Row label="Switching" value={switchingText(services, now) ?? ""} />
          ) : (
            services.modes
              .filter((mode) => mode !== selectedMode(services))
              .map((mode) => (
                <ButtonRow
                  key={mode}
                  label={
                    mode === "llm"
                      ? "Free ComfyUI and load the LLM"
                      : `Switch to ${modeLabel[mode]}`
                  }
                  onPress={() => void requestSwitch(mode)}
                />
              ))
          )}
        </Section>
      ) : null}

      <Section
        title="On this iPhone"
        footer="Covers the app with a blank screen in the app switcher and whenever it isn't in front, so your images don't show over someone's shoulder."
      >
        <View style={styles.row}>
          <Text
            style={styles.rowLabel}
            nativeID="hide-label"
            maxFontSizeMultiplier={maxFontScale}
          >
            Hide in app switcher
          </Text>
          <Switch
            value={hideInSwitcher}
            onValueChange={setHideInSwitcher}
            trackColor={{ true: colors.accent, false: colors.border }}
            accessibilityLabelledBy="hide-label"
          />
        </View>
      </Section>

      <Section
        title="Stored here"
        footer="History and images live on your Mac. This iPhone keeps a cache to show them quickly."
      >
        <ButtonRow label="Clear downloaded images" onPress={clearDownloads} />
        <ButtonRow
          label="Forget this server"
          onPress={forgetServer}
          destructive
        />
      </Section>

      <Section title="About">
        <Row label="App version" value={Constants.expoConfig?.version ?? "—"} />
      </Section>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.background },
  section: { marginTop: 24, paddingHorizontal: 16, gap: 7 },
  sectionTitle: {
    color: colors.muted,
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 0.4,
    paddingHorizontal: 4,
  },
  group: {
    backgroundColor: colors.surface,
    borderRadius: radius.control,
    overflow: "hidden",
  },
  footer: {
    color: colors.muted,
    fontSize: 13,
    lineHeight: 18,
    paddingHorizontal: 4,
  },
  row: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.border,
  },
  pressed: { backgroundColor: colors.card },
  rowLabel: { color: colors.text, fontSize: 16 },
  rowValue: {
    color: colors.muted,
    fontSize: 15,
    flexShrink: 1,
    textAlign: "right",
  },
  button: { color: colors.accent, fontSize: 16 },
  destructive: { color: colors.danger },
});
