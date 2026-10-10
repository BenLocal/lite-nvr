<script setup lang="ts">
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Password from 'primevue/password'
import Select from 'primevue/select'
import { xiaomiRegionOptions, type DeviceFormFields } from '../../forms/deviceForm'

defineProps<{ form: DeviceFormFields }>()
</script>

<template>
  <div class="field-grid">
    <div class="field">
      <label for="xm_user_id">用户 ID</label>
      <InputText
        id="xm_user_id"
        name="xm_user_id"
        class="field-input"
        placeholder="Xiaomi user_id"
        :invalid="form.xm_user_id?.invalid"
      />
      <Message v-if="form.xm_user_id?.invalid" severity="error" size="small" variant="simple">
        {{ form.xm_user_id.error?.message }}
      </Message>
    </div>
    <div class="field">
      <label for="xm_token">Token</label>
      <Password
        id="xm_token"
        name="xm_token"
        class="field-input"
        :feedback="false"
        toggle-mask
        placeholder="passToken"
        :invalid="form.xm_token?.invalid"
      />
      <Message v-if="form.xm_token?.invalid" severity="error" size="small" variant="simple">
        {{ form.xm_token.error?.message }}
      </Message>
    </div>
  </div>

  <div class="field-grid">
    <div class="field">
      <label for="xm_did">设备 DID</label>
      <InputText
        id="xm_did"
        name="xm_did"
        class="field-input"
        placeholder="did"
        :invalid="form.xm_did?.invalid"
      />
      <Message v-if="form.xm_did?.invalid" severity="error" size="small" variant="simple">
        {{ form.xm_did.error?.message }}
      </Message>
    </div>
    <div class="field">
      <label for="xm_model">型号 Model</label>
      <InputText
        id="xm_model"
        name="xm_model"
        class="field-input"
        placeholder="如 chuangmi.camera.xxx"
        :invalid="form.xm_model?.invalid"
      />
      <Message v-if="form.xm_model?.invalid" severity="error" size="small" variant="simple">
        {{ form.xm_model.error?.message }}
      </Message>
    </div>
  </div>

  <div class="field-grid">
    <div class="field">
      <label for="xm_ip">摄像头 IP</label>
      <InputText
        id="xm_ip"
        name="xm_ip"
        class="field-input"
        placeholder="192.168.x.y"
        :invalid="form.xm_ip?.invalid"
      />
      <Message v-if="form.xm_ip?.invalid" severity="error" size="small" variant="simple">
        {{ form.xm_ip.error?.message }}
      </Message>
    </div>
    <div class="field">
      <label for="xm_region">区域</label>
      <Select
        id="xm_region"
        name="xm_region"
        :options="xiaomiRegionOptions"
        option-label="label"
        option-value="value"
        size="small"
        class="field-input"
        placeholder="选择区域"
      />
    </div>
  </div>

  <div class="field">
    <span class="field-hint">
      运行 <code>cargo run -p xiaomi --bin validate</code>
      登录小米账号即可获取 user_id / token，并列出各摄像头的 did / model / ip。
    </span>
  </div>
</template>
