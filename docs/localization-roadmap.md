# UI localization roadmap

## Current support and coverage

The React interface in `webui/` now has a browser-local language selector and a catalog for its fixed, application-owned UI copy. The current source contains 1,142 messages and 13,704 translated locale cells across the twelve supported locales. Each non-English catalog loads on demand as a separate chunk; English remains visible until the selected chunk arrives. Torrent, tracker, category, path, URL, command, and other engine or user-provided values remain data and are not translated.

Catalog coverage is generated from the current WebUI source plus complete templates for dynamic messages and fixed keyboard-help rows. The WebUI unit suite checks exact key parity, non-empty values, and named-placeholder parity in `localeCatalogs.test.ts`. The catalogs were machine-translated and receive automated key, placeholder, non-empty, corruption, and selected terminology checks. These checks do not replace fluent-speaker review.

## Supported locales

The selector and catalog use this order:

1. Spanish (`es`)
2. Simplified Chinese (`zh-Hans`)
3. Hindi (`hi`)
4. German (`de`)
5. Japanese (`ja`)
6. French (`fr`)
7. Brazilian Portuguese (`pt-BR`)
8. Arabic (`ar`)
9. Korean (`ko`)
10. Traditional Chinese (`zh-Hant`)
11. Vietnamese (`vi`)
12. Indonesian (`id`)

## Translation and layout boundaries

The catalog covers fixed labels, help, empty states, validation, notifications, action feedback, stable HTTP error summaries, and application-owned setting and capability descriptions. It includes localized table controls and selection help, storage operation and progress labels, and tag placeholders across bulk edit, RSS rules, and torrent properties. HTTP status and path remain interpolated values; other runtime values remain unchanged. Regional formatting uses the selected locale for supported numbers, dates, times, and sizes.

Translated Arabic labels carry local `lang` and `dir` metadata. The document and untranslated content remain English and left-to-right; this work does not claim app-wide RTL layout support. The catalog and component source receive static checks for completion, storage, direction, and crash-safety terminology.

## Implemented in this PR

The browser-local selector covers all twelve locales. The catalog includes the audited TorrentNG WebUI source, including sign-in, navigation, torrent list and detail, add and edit flows, filters, selection and bulk actions, storage and crash-safety settings, engine configuration, RSS rules, workflows, tracker health, user-agent settings, help, empty and error states, accessibility labels, and dynamic messages.

Regional number formatting follows live interface-language changes. Arabic direction metadata is applied only to translated labels. Browser storage failure keeps the current tab usable, and cross-tab preference updates remain supported.

## Follow-up

Check Arabic label placement in real layouts before making any broader RTL claim. Extend the inventory and catalog checks whenever new fixed UI strings are added.

## Model-assisted follow-up — 2026-10-30

Automated catalog invariants cover all 13,704 direct translation cells. A focused pass reviewed 108 cells across nine high-risk state, action, glossary, and keyboard-help messages in all twelve locales, correcting 73 values. A systematic contextual pass then reviewed the first 60 source keys in alphabetical order (720 cells), all 36 translated values for the Settings, Backend, and Crash safety navigation labels, and one Hindi restore-button reference. In total, 865 unique cells have received contextual review and 114 values were corrected; the other 12,839 cells have not received that pass. No qualified human-review status is claimed; fluent-speaker review remains outstanding.
