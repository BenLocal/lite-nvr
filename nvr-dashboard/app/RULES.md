# Frontend Rules

## 1. API Request Organization

- All HTTP requests must be placed under the `src/api` module.
- API files must be split by business domain/module.
- Example: user-related requests should be implemented in `src/api/user.ts`.
- Components, views, and composables must not directly write request logic.
- Shared request behaviors (base URL, auth headers, error handling) must be implemented in a common request utility (for example `src/api/request.ts`).

## 2. PrimeVue Usage Consistency

- Prefer using PrimeVue official components and MCP where applicable.
- For similar UI patterns, use related PrimeVue components to keep a consistent visual style.
- PrimeVue MCP server should use the following configuration:

```json
{
  "servers": {
    "primevue": {
      "command": "npx",
      "args": ["-y", "@primevue/mcp"]
    }
  }
}
```

## 3. Form Validation Style

- Use `@primevue/forms` as the default form validation solution.
- Form fields with validation errors must use component `invalid` state so the input border turns red.
- Validation error text should be rendered under the corresponding field using PrimeVue style (`Message` with small/simple style is preferred).

## 4. Dialog and Notification Style

- Do not use browser-native dialogs such as `window.alert`, `window.confirm`, or direct `alert()`/`confirm()`.
- Confirmation interactions must use `primevue/confirmdialog`.
- Warning/info/success feedback must use `primevue/toast`.
- Keep `ConfirmDialog` and `Toast` mounted at app root and use service hooks (`useConfirm`, `useToast`) in pages/components.

## 5. Form Field Width Consistency

- Use `class="field-input"` for form controls that should fill available width.
- For wrapped PrimeVue inputs (for example `Password`, `InputNumber`, `DatePicker`, `Select`, `MultiSelect`, `AutoComplete`), keep width behavior aligned with `InputText` by ensuring both wrapper and inner input are `width: 100%`.
- Keep width rules centralized in app-level global styles (for example `App.vue`). Do not duplicate the same `field-input` width rules inside individual views.

## 6. Shared View Style Reuse

- For repeated page patterns (for example content card container, title/subtitle/content spacing), extract shared styles into a common stylesheet under `src/styles`.
- Views should keep only page-specific styles; avoid copying the same card/layout style blocks across multiple view files.
- Prefer PrimeVue theme CSS variables (for example `var(--p-content-border-color)`) as the single source of truth. Avoid mixing literal fallback colors unless compatibility explicitly requires it.

## 7. Request Header Typing Safety

- In shared request utilities, do not spread `RequestInit.headers` directly into object literals.
- Normalize headers with `new Headers(headers)` before setting defaults (for example `Content-Type`) and auth headers.
- This keeps behavior consistent across `Headers`, tuple arrays, and plain object forms of `HeadersInit`, and avoids TypeScript union type conflicts.

## 8. Implementation Templates

- Shared content-card style template (`src/styles/content-card.css`):

```css
.content-section {
  max-width: 100%;
}

.content-card {
  margin-bottom: 1rem;
  border: 1px solid var(--p-content-border-color);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
}

.content-card .p-card-title {
  font-size: 1.125rem;
}

.content-card .p-card-subtitle {
  font-size: 0.8125rem;
}

.content-card .p-card-content {
  padding: 0.75rem 0;
}
```

- Global field width rule template (`App.vue` global style):

```css
.field-input {
  width: 100%;
}

.p-password.field-input,
.p-inputnumber.field-input,
.p-datepicker.field-input,
.p-select.field-input,
.p-multiselect.field-input,
.p-autocomplete.field-input {
  width: 100%;
}

.p-password.field-input .p-password-input,
.p-inputnumber.field-input .p-inputnumber-input,
.p-datepicker.field-input .p-inputtext,
.p-select.field-input .p-select-label,
.p-multiselect.field-input .p-multiselect-label,
.p-autocomplete.field-input .p-inputtext {
  width: 100%;
}
```

- Request header normalization template (`src/api/request.ts`):

```ts
const requestHeaders = new Headers(headers)
requestHeaders.set('Content-Type', 'application/json')
if (token) {
  requestHeaders.set('Authorization', `Bearer ${token}`)
}

await fetch(url, {
  ...rest,
  method,
  headers: requestHeaders,
  body: body === undefined ? undefined : JSON.stringify(body),
})
```

## 9. Lint Command Convention

- `npm run lint`: run both ESLint and Stylelint checks (no auto-fix), suitable for CI.
- `npm run format`: run ESLint + Stylelint auto-fix for local cleanup.
- Do not rely on a single linter when submitting frontend changes.

## 10. CI Validation Convention

- Frontend CI workflow should run in `nvr-dashboard/app` and execute:
  1. `npm ci`
  2. `npm run lint`
  3. `npm run type-check`
- Keep frontend CI path-filtered to frontend scope to avoid unrelated pipeline noise.
