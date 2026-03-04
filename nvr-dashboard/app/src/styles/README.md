# Styles Guide

This folder stores shared frontend styles for reusable view patterns.

## Scope

- Put cross-view visual patterns here (for example content cards, shared layout blocks).
- Keep view-local styling in each `.vue` file when the style is page-specific.
- Use PrimeVue theme variables as the primary source of truth.

## Current Shared Files

- `content-card.css`: shared card container/title/subtitle/content spacing styles used by dashboard-like pages.

## Usage

1. Import shared stylesheet once in app entry:

```ts
// src/main.ts
import './styles/content-card.css'
```

2. In view files, use shared class names and keep only page-specific styles:

```vue
<template>
  <div class="content-section">
    <Card class="content-card">
      <template #title>Title</template>
      <template #subtitle>Subtitle</template>
      <template #content>
        <p class="page-specific-text">...</p>
      </template>
    </Card>
  </div>
</template>

<style scoped>
.page-specific-text {
  margin: 0;
}
</style>
```

## Form Width Rule

Form width behavior is centralized in global style (`App.vue`) through `field-input`.
Do not duplicate these width rules inside individual views.

## Quick Checklist For New Pages

1. Reuse shared classes from `src/styles` before writing new layout/card styles.
2. If two or more pages share the same style block, extract it to this folder.
3. Use `var(--p-*)` tokens instead of hard-coded colors whenever possible.
4. Keep page styles focused on page-specific behavior only.
