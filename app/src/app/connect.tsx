import * as Haptics from 'expo-haptics';
import { router, Stack } from 'expo-router';
import { useRef, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Linking,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Icon } from '../components/Icon';
import { normalizeAddress } from '../lib/address';
import { testConnection } from '../lib/api';
import { useConnection } from '../lib/connection';
import { colors, maxFontScale, radius } from '../lib/theme';

const TAILSCALE_APP_STORE = 'https://apps.apple.com/app/tailscale/id1470499037';

type Problem = { message: string; field?: 'address' | 'password' };

/**
 * The one-time connection setup: the Mac's address and the password chosen there. Both are
 * checked against the server before they are saved to the Keychain.
 */
export default function Connect() {
  const insets = useSafeAreaInsets();
  const { connection, rejected, save } = useConnection();
  const firstRun = !connection;
  const [address, setAddress] = useState(connection?.address ?? '');
  // After the Mac's password changed, the old one is no use: start empty.
  const [password, setPassword] = useState(rejected ? '' : (connection?.password ?? ''));
  const [checking, setChecking] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);
  const passwordField = useRef<TextInput>(null);

  const normalized = normalizeAddress(address);
  const canConnect = !checking && address.trim().length > 0 && password.length > 0;

  const connect = async () => {
    if (!canConnect) return;
    if (!normalized) {
      setProblem({ field: 'address', message: 'That is not an address. Use the https://… address your Mac prints when the server starts.' });
      return;
    }
    setChecking(true);
    setProblem(null);
    const result = await testConnection({ address: normalized, password });
    setChecking(false);
    if (!result.ok) {
      setProblem(result);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      return;
    }
    await save({ address: normalized, password });
    void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    if (router.canGoBack()) router.back();
    else router.replace('/');
  };

  return (
    <KeyboardAvoidingView style={styles.screen} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <Stack.Screen options={{ headerShown: !firstRun }} />
      <ScrollView
        contentContainerStyle={[styles.content, { paddingTop: firstRun ? insets.top + 28 : 20, paddingBottom: insets.bottom + 24 }]}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="interactive"
      >
        {firstRun ? (
          <View style={styles.intro}>
            <Text style={styles.title} maxFontSizeMultiplier={maxFontScale} accessibilityRole="header">
              Connect to your Mac
            </Text>
            <Text style={styles.lead} maxFontSizeMultiplier={maxFontScale}>
              Prompt sends what you type to the ComfyUI prompt server on your Mac and shows the images it makes. Nothing goes anywhere else.
            </Text>
          </View>
        ) : null}

        <View style={styles.step}>
          <Text style={styles.stepTitle} maxFontSizeMultiplier={maxFontScale}>
            Tailscale on this iPhone
          </Text>
          <Text style={styles.stepText} maxFontSizeMultiplier={maxFontScale}>
            Install Tailscale, sign in with the same account as your Mac, and keep it connected while you use Prompt.
          </Text>
          <Pressable onPress={() => Linking.openURL(TAILSCALE_APP_STORE)} accessibilityRole="link" style={styles.linkRow} hitSlop={6}>
            <Text style={styles.link} maxFontSizeMultiplier={maxFontScale}>
              Get Tailscale on the App Store
            </Text>
            <Icon name="arrow.up.forward" size={13} color={colors.accent} fallback="↗" />
          </Pressable>
        </View>

        <View style={styles.fieldGroup}>
          <Text style={styles.label} nativeID="address-label" maxFontSizeMultiplier={maxFontScale}>
            Server address
          </Text>
          <TextInput
            style={[styles.field, problem?.field === 'address' && styles.fieldProblem]}
            value={address}
            onChangeText={(text) => {
              setAddress(text);
              setProblem(null);
            }}
            placeholder="https://my-mac.tailnet-name.ts.net"
            placeholderTextColor={colors.faint}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            textContentType="URL"
            returnKeyType="next"
            onSubmitEditing={() => passwordField.current?.focus()}
            editable={!checking}
            accessibilityLabelledBy="address-label"
            keyboardAppearance="dark"
            maxFontSizeMultiplier={maxFontScale}
          />
          <Text style={styles.hint} maxFontSizeMultiplier={maxFontScale}>
            {normalized && normalized !== address.trim()
              ? `Will connect to ${normalized}`
              : 'Your Mac prints it when the server starts, on the line that says "Open … (from your Tailscale devices)".'}
          </Text>
        </View>

        <View style={styles.fieldGroup}>
          <Text style={styles.label} nativeID="password-label" maxFontSizeMultiplier={maxFontScale}>
            Password
          </Text>
          <TextInput
            ref={passwordField}
            style={[styles.field, problem?.field === 'password' && styles.fieldProblem]}
            value={password}
            onChangeText={(text) => {
              setPassword(text);
              setProblem(null);
            }}
            placeholder="The one you chose on your Mac"
            placeholderTextColor={colors.faint}
            secureTextEntry
            autoCapitalize="none"
            autoCorrect={false}
            textContentType="password"
            autoComplete="current-password"
            returnKeyType="go"
            onSubmitEditing={connect}
            editable={!checking}
            accessibilityLabelledBy="password-label"
            keyboardAppearance="dark"
            maxFontSizeMultiplier={maxFontScale}
          />
          <Text style={styles.hint} maxFontSizeMultiplier={maxFontScale}>
            Saved in this iPhone’s Keychain and sent only to your Mac, over Tailscale’s encrypted connection.
          </Text>
        </View>

        {problem ? (
          <View style={styles.problem} accessibilityRole="alert">
            <Icon name="exclamationmark.triangle.fill" size={16} color={colors.danger} fallback="!" />
            <Text style={styles.problemText} maxFontSizeMultiplier={maxFontScale}>
              {problem.message}
            </Text>
          </View>
        ) : null}

        <Pressable
          onPress={connect}
          disabled={!canConnect}
          accessibilityRole="button"
          accessibilityState={{ disabled: !canConnect, busy: checking }}
          style={({ pressed }) => [styles.button, !canConnect && styles.buttonOff, pressed && styles.buttonPressed]}
        >
          {checking ? <ActivityIndicator color={colors.accentText} /> : null}
          <Text style={styles.buttonText} maxFontSizeMultiplier={maxFontScale}>
            {checking ? 'Checking server…' : firstRun ? 'Connect' : 'Save'}
          </Text>
        </Pressable>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.background },
  content: { paddingHorizontal: 20, gap: 22 },
  intro: { gap: 10 },
  title: { color: colors.text, fontSize: 30, fontWeight: '700' },
  lead: { color: colors.muted, fontSize: 16, lineHeight: 22 },
  step: { backgroundColor: colors.surface, borderRadius: radius.card, padding: 16, gap: 6 },
  stepTitle: { color: colors.text, fontSize: 16, fontWeight: '600' },
  stepText: { color: colors.muted, fontSize: 14, lineHeight: 20 },
  linkRow: { flexDirection: 'row', alignItems: 'center', gap: 5, marginTop: 4 },
  link: { color: colors.accent, fontSize: 15, fontWeight: '600' },
  fieldGroup: { gap: 7 },
  label: { color: colors.text, fontSize: 15, fontWeight: '600' },
  field: {
    backgroundColor: colors.field,
    color: colors.text,
    fontSize: 16,
    paddingHorizontal: 14,
    paddingVertical: 12,
    borderRadius: radius.control,
    borderWidth: 1,
    borderColor: colors.border,
  },
  fieldProblem: { borderColor: colors.danger },
  hint: { color: colors.muted, fontSize: 13, lineHeight: 18 },
  problem: { flexDirection: 'row', gap: 10, alignItems: 'flex-start', backgroundColor: colors.surface, padding: 14, borderRadius: radius.control },
  problemText: { flex: 1, color: colors.text, fontSize: 14, lineHeight: 20 },
  button: {
    flexDirection: 'row',
    gap: 10,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: colors.accent,
    borderRadius: radius.control,
    minHeight: 50,
    paddingHorizontal: 16,
  },
  buttonOff: { opacity: 0.4 },
  buttonPressed: { opacity: 0.8 },
  buttonText: { color: colors.accentText, fontSize: 17, fontWeight: '700' },
});
