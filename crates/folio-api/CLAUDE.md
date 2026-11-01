# folio-api

## Askama templates

- Use `{% match %}` blocks for Option handling (not closures)
- For string comparisons in templates: `{% let id = item.id.to_string() %}` then compare with `id.as_str() == other.as_str()`
- Form handlers in page routes return `Redirect::to()` for POST→GET redirects

## Turbo + Stimulus

- Stimulus controllers are defined inline in `base.html`
- To disable Turbo on forms: `data-turbo="false"` and use `this.form.requestSubmit()` (not `submit()`) for proper event handling
- AJAX in Stimulus: use `fetch()` with form data, check `response.ok || response.redirected` for success
