# Spec Delta

## ADDED Requirements

### Requirement: Default vault preference

Each user SHALL choose their default vault from the vaults where they have a role, on `/settings` (HTML, answered with a Turbo Stream) or with `PUT /api/me/default-vault` and `{"vault_id"}` (204). The choice SHALL be stored per user, not in `config.toml` or the `settings` table.

#### Scenario: Pick a default on the settings page

- **WHEN** Alice selects `Business` as her default vault on `/settings` and saves
- **THEN** opening `/` redirects her to `/vaults/{business}`

#### Scenario: Vault without a role

- **WHEN** Alice sends `PUT /api/me/default-vault` with the ID of a vault where she has no role
- **THEN** the response status is 404 and her default vault is unchanged

#### Scenario: Default per user

- **WHEN** Alice picks `Business` as her default
- **THEN** Bob's default vault is unchanged

### Requirement: AI settings are instance-wide

The AI provider, API keys and model settings SHALL apply to every vault; vaults SHALL NOT override them.

#### Scenario: Provider change applies to all vaults

- **WHEN** an admin switches the provider to `openai` on `/settings`
- **THEN** OCR and search in every vault use `openai` within 5 seconds

#### Scenario: Local provider

- **WHEN** the provider is `local`
- **THEN** no AI provider is called for OCR or embeddings in any vault
