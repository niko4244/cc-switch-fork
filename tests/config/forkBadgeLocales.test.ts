import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import zh from "@/i18n/locales/zh.json";

// Component tests run with empty translations and only ever see ForkBuildBadge's
// defaultValue strings, so a missing key or a dropped placeholder in a locale
// file would pass them. This pins what the badge reads.
const requiredKeys = [
  "fork.badgeLabel",
  "fork.badgeTitle",
  "fork.statePinned",
  "fork.stateOfficial",
  "fork.stateOfficialPending",
] as const;

const requiredPlaceholders: Record<string, string[]> = {
  "fork.badgeLabel": ["{{commit}}"],
  "fork.badgeTitle": ["{{build}}", "{{state}}"],
};

type TranslationTree = Record<string, unknown>;

function readTranslation(tree: TranslationTree, path: string): unknown {
  return path.split(".").reduce<unknown>((value, segment) => {
    if (typeof value !== "object" || value === null) return undefined;
    return (value as TranslationTree)[segment];
  }, tree);
}

describe("fork build badge locale coverage", () => {
  it.each([
    ["zh", zh],
    ["zh-TW", zhTW],
    ["en", en],
    ["ja", ja],
  ])(
    "defines every badge key with its placeholders in %s",
    (_locale, translations) => {
      const problems = requiredKeys.flatMap((key) => {
        const value = readTranslation(translations, key);
        if (typeof value !== "string" || value.trim().length === 0) {
          return [`${key}: missing`];
        }
        return (requiredPlaceholders[key] ?? [])
          .filter((placeholder) => !value.includes(placeholder))
          .map((placeholder) => `${key}: no ${placeholder}`);
      });

      expect(problems).toEqual([]);
    },
  );
});
