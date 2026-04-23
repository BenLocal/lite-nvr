<script setup lang="ts">
import ConfirmDialog from 'primevue/confirmdialog'
import Toast from 'primevue/toast'
</script>

<template>
  <ConfirmDialog />
  <Toast position="bottom-right" />
  <RouterView />
</template>

<style>
* {
  box-sizing: border-box;
}

body {
  margin: 0;
  padding: 0;
  font-family: -apple-system, BlinkMacSystemFont, Inter, 'SF Pro Display', sans-serif;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

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

/* Custom scrollbar for dark theme */
::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-track {
  background: rgb(15 23 42 / 40%);
}

::-webkit-scrollbar-thumb {
  background: rgb(148 163 184 / 30%);
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: rgb(148 163 184 / 50%);
}
</style>
