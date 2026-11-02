---
category: fixed
audience: users
area: webui
action: none
breaking: false
---
WebUI translations now use clearer wording for torrent states, storage checks, settings paths, actions, and keyboard help across all twelve supported languages.

The Hindi `Tracker contains` filter label now means that the tracker contains the entered text. French global download and upload slot labels use the feminine form `globale` to agree with `limite`.

Dynamic values inserted into translated UI templates are isolated with `<bdi dir="auto">`, including mixed-script values inside Arabic text. The Swarm Switch and sign-in source strings now flow through the reactive localization component. Their 79 UI source strings (948 locale cells) remain explicitly pending translation; no filler values were added.
