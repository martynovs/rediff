# theming and config

## Purpose

Provide a curated set of built-in themes whose UI chrome follows the active theme, and read/persist user preferences (theme, layout, …) from the config file, with CLI flags overriding config for a given run.
## Requirements
### Requirement: Built-in themes
The system SHALL provide a curated set of built-in themes drawn from the bundled `two-face` theme collection, including both dark and light themes, and SHALL let the user switch the active theme at runtime. The UI chrome SHALL follow the active theme (foreground, muted, selection, and header/selection backgrounds derived from the theme), except for the diff add/del colors and the local/commit source accents.

#### Scenario: Switch theme at runtime
- **WHEN** the user opens the theme selector and chooses a different built-in theme
- **THEN** the UI re-renders with the selected theme's colors, including chrome derived from that theme

#### Scenario: Multiple themes available
- **WHEN** the user opens the theme selector
- **THEN** more than two themes are offered (the adopted collection), not only a single dark and light option

#### Scenario: Diff colors fall back to a standard palette
- **WHEN** the active theme does not define both inserted and deleted diff colors
- **THEN** the diff add/del foregrounds use a standard green/red, and their backgrounds are blended toward the active theme's background so they remain readable

#### Scenario: Source accents stay standard
- **WHEN** the active theme changes
- **THEN** the local (staged) and commit source accents remain their standard blue/green so the kind-of-diff signal stays recognizable

### Requirement: Configuration file
The system SHALL read persisted preferences from `~/.config/rediff/config.toml`, including at least theme, layout mode, line numbers, and line wrapping, and SHALL be able to write the theme preference back to that file while preserving existing keys and comments.

#### Scenario: Config applied at startup
- **WHEN** a config file sets a theme and layout mode
- **THEN** the system starts with those preferences applied

#### Scenario: Missing config uses defaults
- **WHEN** no config file is present
- **THEN** the system starts with built-in defaults and does not error

#### Scenario: Committed theme is persisted
- **WHEN** the user commits a theme in the picker
- **THEN** the theme key in the config file is updated, other keys and comments are preserved, and the directory/file is created if absent

#### Scenario: Persist failure does not crash
- **WHEN** writing the config file fails (e.g. read-only filesystem)
- **THEN** the in-session theme still applies and the failure is surfaced without crashing the application

### Requirement: CLI flags override config
Command-line flags SHALL override the corresponding config-file values for a given invocation.

#### Scenario: Flag beats config
- **WHEN** the config sets one layout mode and the user passes a different mode flag
- **THEN** the flag's mode is used for that invocation

### Requirement: Legacy theme names remain valid
The system SHALL continue to accept the legacy `"dark"` and `"light"` theme values in the config file and CLI, mapping each to a corresponding theme in the adopted collection.

#### Scenario: Legacy dark value
- **WHEN** the config or CLI specifies the theme `"dark"`
- **THEN** the system starts in a dark theme from the adopted collection without error

#### Scenario: Unknown theme name
- **WHEN** the config or CLI specifies a theme name that is not in the collection
- **THEN** the system falls back to a default theme without error

### Requirement: Verdict presets
The system SHALL read named parting instructions ("verdict presets") from `config.toml` as a
top-level array of tables, each with a name and its text, and SHALL offer them when the user closes
a review round. When the file configures none, the system SHALL offer built-in defaults, so a
configuration can never leave the user without a way to close a round.

#### Scenario: Configured presets are offered in file order
- **WHEN** the config file declares verdict presets
- **THEN** those presets are offered for selection, in the order they appear in the file

#### Scenario: No configured presets falls back to the built-ins
- **WHEN** the config file declares no verdict presets, or declares an empty list
- **THEN** the built-in presets are offered

### Requirement: One malformed key costs only itself
The system SHALL read each configuration key independently, so a value the system cannot parse
discards only that key and leaves every other preference in effect.

#### Scenario: A malformed preset does not discard the theme
- **WHEN** the config file sets a theme and a layout mode, and also contains a verdict preset the
  system cannot parse
- **THEN** the theme and layout mode are applied, and only the presets fall back to the built-ins
