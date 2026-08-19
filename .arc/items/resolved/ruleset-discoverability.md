When the selected ruleset resolves to nothing — a misconfigured custom ruleset, an empty sources list, a bare `--rules` directory — the rules surfaces (CLI and TUI) say so and name the built-in `default` as the always-available fallback, instead of dead-ending at an empty state. (The built-in `default` itself always resolves; the snag is discovery at the moment a custom selection comes up empty.) A known stranger-path snag, not speculative polish.

---
Resolved: landed as `rules::BUILTIN_HINT` — every empty resolution (CLI `arc rules` and the TUI rules view, one shared line) names the built-in `default` and the `builtin` source instead of dead-ending, with the project-override qualification worded in.
