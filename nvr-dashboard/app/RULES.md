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
