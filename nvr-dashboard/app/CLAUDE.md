# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Summary

Vue 3 + TypeScript frontend for the lite-nvr web dashboard. Built with Vite, PrimeVue UI library, and Vue Router. Embedded into the Rust binary via `rust-embed` and served at `/nvr/`.

## Development Commands

```bash
# Install dependencies
npm ci

# Development server with hot reload (proxies /api to localhost:18080)
npm run dev

# Build for production (sets base URL to /nvr/)
npm run build

# Type checking
npm run type-check

# Lint (ESLint + Stylelint, no auto-fix)
npm run lint

# Format (ESLint + Stylelint with auto-fix)
npm run format

# Individual linters
npm run lint:eslint
npm run lint:style
```

## Architecture

**Directory structure:**

```
src/
├── api/          # HTTP request modules, organized by business domain
├── auth/         # Authentication utilities (token storage)
├── composables/  # Vue composables (useAuth, etc.)
├── components/   # Reusable Vue components
├── layouts/      # Layout components (AppLayout with auth guard)
├── views/        # Page components (LoginView, DashboardView, etc.)
├── router/       # Vue Router configuration with auth guards
├── styles/       # Shared stylesheets and PrimeVue theme preset
└── main.ts       # App entry point, PrimeVue setup
```

**Key files:**
- `src/main.ts` — PrimeVue configuration with Aura theme, global component sizing (sm), ConfirmationService, ToastService
- `src/router/index.ts` — Routes with auth guards (`requiresAuth`, `guest` meta)
- `src/api/request.ts` — Shared fetch wrapper with auth headers and error handling
- `vite.config.ts` — Base URL set to `/nvr/`, dev server proxies `/api` to `localhost:18080`

**Authentication flow:**
1. User logs in via `LoginView` → calls `src/api/user.ts`
2. Token stored in localStorage via `src/auth/token.ts`
3. Router guard in `src/router/index.ts` checks `useAuth().isLoggedIn`
4. Protected routes redirect to `/login` if not authenticated
5. All API requests include `Authorization: Bearer <token>` header via `src/api/request.ts`

## PrimeVue Usage

**IMPORTANT: Always use PrimeVue components for all UI elements. Do NOT create custom components when PrimeVue provides an equivalent.**

**Official documentation for LLMs:** https://primevue.org/llms/llms-full.txt

Before implementing any UI component, ALWAYS:
1. Check the PrimeVue LLM documentation at https://primevue.org/llms/llms-full.txt
2. Use the official PrimeVue component if available
3. Only create custom components when PrimeVue does not provide the functionality

**Component priority:**
- Buttons → `primevue/button`
- Forms → `@primevue/forms` with PrimeVue input components
- Tables → `primevue/datatable`
- Dialogs → `primevue/dialog`
- Notifications → `primevue/toast`
- Confirmations → `primevue/confirmdialog`
- Tabs → `primevue/tabs`
- Cards → `primevue/card`
- Menus → `primevue/menu`
- Dropdowns → `primevue/select`
- Date pickers → `primevue/datepicker`
- File uploads → `primevue/fileupload`
- Progress → `primevue/progressbar`, `primevue/progressspinner`
- Charts → `primevue/chart`
- And many more - always check the documentation first

**Key conventions:**
- Use PrimeVue components consistently for UI patterns
- Forms use `@primevue/forms` for validation
- Dialogs use `primevue/confirmdialog` (never `window.confirm`)
- Notifications use `primevue/toast` (never `window.alert`)
- Global component sizing: all components use `sm` size via PassThrough in `main.ts`
- Theme: Aura preset from `@primeuix/themes/aura`

**Form field width:**
- Use `class="field-input"` for full-width form controls
- Global styles in `App.vue` ensure wrapped components (Password, Select, etc.) fill width
- Do not duplicate width rules in individual views

**Validation style:**
- Invalid fields use component `invalid` prop (red border)
- Error messages rendered below field using `Message` component with small/simple style

## API Request Organization

**Rules:**
- All HTTP requests in `src/api/` module, split by business domain
- Components/views/composables must NOT directly write request logic
- Shared request behavior (base URL, auth, error handling) in `src/api/request.ts`

**Request header handling:**
- Normalize headers with `new Headers(headers)` before setting defaults
- Avoids TypeScript union type conflicts with `HeadersInit`
- See `src/api/request.ts` for template

## Styling Conventions

**Shared styles:**
- Extract repeated patterns (content cards, spacing) to `src/styles/`
- Views keep only page-specific styles
- Use PrimeVue theme CSS variables: `var(--p-content-border-color)`, etc.
- Avoid literal color fallbacks unless compatibility requires it

## Theme Style Guide

When adding or modifying any page in `nvr-dashboard/app`, keep the UI aligned with the existing dashboard theme. Do not introduce a new visual language for individual pages.

**Overall visual direction:**
- Dark control-room style, based on slate/navy backgrounds with translucent glass panels
- Accent color is blue/cyan (`#3b82f6`, `#38bdf8`), not purple
- Surfaces should feel layered and slightly luminous, not flat white or flat gray
- Typography should stay compact, operational, and information-dense rather than marketing-style

**Core palette and usage:**
- Primary panel background: `rgba(15, 23, 42, 0.4)` or close variants
- Secondary panel/table background: `rgba(30, 41, 59, 0.3~0.6)`
- Main text: `#e2e8f0`
- Secondary text: `#cbd5e1`
- Muted text / metadata: `#94a3b8`, `#64748b`
- Border color: `rgba(148, 163, 184, 0.08~0.12)`
- Primary emphasis / active state: blue family, especially `#3b82f6`

**Layout primitives to reuse first:**
- Reuse `.content-section`, `.page-header`, `.header-content`, `.page-title`, `.page-subtitle`, `.page-actions` from `src/styles/global-dark-theme.css`
- Reuse `.data-card` and `.content-card` from `src/styles/global-dark-theme.css`
- Reuse `.empty-state`, `.empty-state-icon`, `.empty-state-text`
- Reuse typography helpers like `.mono-text`, `.single-line-text`, `.ellipsis-text`

**Card style rules:**
- Default business panels should use `.data-card` unless there is a strong reason not to
- Card look should stay consistent: blur background, subtle border, rounded corners, soft shadow
- Typical values:
  - `backdrop-filter: blur(12px)`
  - `border-radius: 0.75rem`
  - `border: 1px solid rgba(148, 163, 184, 0.1)`
  - `box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2)`
- Page-specific animation delay can stay local in the view, but base card appearance should be shared

**Table style rules:**
- Prefer PrimeVue `DataTable`
- Base table look should follow `src/styles/global-dark-theme.css`
- Header, row, hover, empty, and loading states must stay in the same dark-glass family as cards
- Do not add bright table headers, white cells, or strong grid lines
- If a page needs table-specific overrides, keep them local and preserve the same palette and border softness

**Empty state rules:**
- Empty states should appear as part of the same surface system, not plain text dropped into the page
- Prefer icon + primary message + optional hint message
- If there is no data, consider rendering a standalone empty state instead of leaving table headers visible

**Motion and interaction:**
- Use existing `fadeIn` / `slideUp` motion language
- Motion should be restrained and functional: subtle hover lift, subtle shadow increase, smooth opacity transitions
- Avoid bouncy, flashy, or colorful animations

**What to avoid:**
- White backgrounds
- Purple gradients or unrelated accent palettes
- Sharp black borders
- Large marketing-style hero sections inside management views
- Re-defining global helper classes inside view-local `<style scoped>` blocks
- Copy-pasting the same `.data-card`, empty-state, or utility styles into multiple views instead of extracting them

**Extraction rules:**
- If the same style block appears in 2 or more views, move it to `src/styles/`
- Before writing new page-local styles, check:
  - `src/styles/global-dark-theme.css`
  - `src/styles/prime-preset.ts`
  - `src/styles/README.md`
- View-local styles should contain only page-specific structure or one-off visual behavior

**Implementation expectation:**
- New pages and modified pages should look like they belong to the same product as `DashboardView.vue`, `DeviceListView.vue`, and `PlaybackView.vue`
- If a new UI element breaks the existing theme, adjust the new code to the theme instead of creating a separate style island

**Content card template** (`src/styles/global-dark-theme.css`):
```css
.content-card {
  margin-bottom: 1rem;
  border: 1px solid var(--p-content-border-color);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
}
```

## Build Integration

- Frontend is automatically built by `nvr-dashboard/build.rs` during `cargo build`
- Build output (`dist/`) is embedded into Rust binary via `rust-embed`
- Base URL is `/nvr/` (set via `VITE_BASE_URL` in package.json build script)
- Dev server proxies `/api` to backend at `localhost:18080`

## CI Validation

Frontend CI should run:
1. `npm ci`
2. `npm run lint` (ESLint + Stylelint)
3. `npm run type-check`

Path-filter CI to frontend scope to avoid unrelated pipeline noise.

## Pre-Commit Checks

Before committing any changes under `nvr-dashboard/app`, run:

```bash
npm run lint
```

Do not commit frontend changes while ESLint or Stylelint errors remain. If style errors are mostly mechanical, run `npm run lint:style:fix` first, then re-run `npm run lint`.

## Node Version

Requires Node.js `^20.19.0 || >=22.12.0` (see `package.json` engines field).
