## ADDED Requirements

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
