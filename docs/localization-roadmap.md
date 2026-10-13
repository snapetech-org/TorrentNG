# UI localization roadmap

## Existing support and verified gap

The current React interface is under `webui/`. The reachable source, branches, and pull-request histories contain no app-level UI catalogs, locale selector, or localization attempt. Several dates and counts already use browser `toLocale*` formatting; interface labels and messages remain English.

## Implementation plan

1. Extract visible copy from `webui/src` into an English source catalog. Preserve torrent names, tracker names, categories, paths, URLs, and values returned by the engine.
2. Extend the compact shell catalog with screen-sized batches and English per-key fallback. Add a full message framework when dynamic errors or plural rules make it useful, with missing-key and placeholder checks.
3. Put Display Language in the existing Settings > Appearance panel. Persist it with the same per-browser preferences as appearance, apply it immediately, and use the browser's preferred locale only before the user makes an explicit choice.
4. Drive date, time, count, and plural formatting from that saved locale so labels and formatted values stay consistent.
5. Add locales in the ranked order below. Arabic needs document `lang`/ `dir`, logical layout rules, and RTL review of the torrent table, selection controls, dialogs, and settings at mobile and desktop widths.

## First-pass locale order

This is a practical rollout order, not a claim about these apps' actual user base. Product-level language analytics are not available. It combines global speaker estimates with online-content usage, and keeps the requested Arabic and Asian-language coverage in the first waves:

1. Spanish (es; es-MX where the product already distinguishes it)
2. Simplified Chinese (zh-Hans / zh-CN)
3. Hindi (hi)
4. German (de)
5. Japanese (ja)
6. French (fr)
7. Brazilian Portuguese (pt-BR)
8. Arabic (ar; requires right-to-left layout support)
9. Korean (ko)
10. Traditional Chinese (zh-Hant / zh-TW)
11. Vietnamese (vi)
12. Indonesian (id)

The World Bank's 2025 report estimates Mandarin Chinese, Hindi, and Spanish at 15%, 7.6%, and 6.9% of the global population respectively; its online-content data also places German, Chinese, and Japanese among the leading languages. [World Bank, Digital Progress and Trends Report 2025](https://documents1.worldbank.org/curated/en/099112525160536089/pdf/P505350-59c98ca8-0803-4f23-b470-17f3dab010ab.pdf). W3Techs tracks website content-language share rather than app users. [W3Techs content-language survey](https://w3techs.com/technologies/cross/content_language%20/ranking). Reorder this list if maintainers later receive reliable demand signals.

## Translation boundaries and acceptance

Translate interface labels, help, empty states, validation, notifications, and errors. Keep media titles, user names, filenames and paths, commands, API identifiers, protocol values, and other user-supplied content unchanged. Use the selected UI language for date, time, number, and plural formatting. Every locale must fall back to English for a missing key; CI should check placeholder parity and report missing strings. Arabic translated labels use local `lang` and `dir` metadata. App-wide RTL still requires logical layout properties and review of core workflows at narrow and desktop widths.

## Implemented in this PR

Settings > Support > Appearance offers a browser-local interface-language selector for twelve locales. The app tabs, settings navigation, and language-control help use translated labels with English fallback. Arabic translated controls carry local language and direction metadata; untranslated workflows and the document remain English and left-to-right.

Regional formatting now updates open screens and applies localized digits and separators to torrent sizes, ratios, progress, storage usage, speeds, and counts across the main list, details, selection, storage, and status surfaces. The preference remains active in the current tab if browser storage is unavailable and follows changes from other tabs. Sticky table columns use logical inline positioning.

## Remaining work

The Add Torrent dialog now translates its file staging, URL, category, save-path, start/pause, validation, and submit controls in the first-wave catalogs. Torrent names, URLs, and paths remain unchanged; English fallback remains available and Arabic labels receive local direction metadata. Translate the remaining list/detail workflows, broader errors, and help copy in later screen-sized batches. Complete Arabic layout review across dialogs, filters, tables, and settings before claiming app-wide RTL support.
