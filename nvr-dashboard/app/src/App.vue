<script setup lang="ts">
import ConfirmDialog from 'primevue/confirmdialog'
import Toast from 'primevue/toast'
</script>

<template>
  <ConfirmDialog />
  <Toast position="top-right" />
  <RouterView />
</template>

<style>
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
</style>
