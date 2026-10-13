<script setup lang="ts">
import { computed } from 'vue'
import Tag from 'primevue/tag'
import type { CodecMetrics, DeviceItem, DeviceMetrics, MediaStreamMetrics } from '../api/device'

import type { AudioSourceOption, MixerState, ProgramState } from '../api/audiomixer'

const props = defineProps<{
  metrics?: DeviceMetrics
  stale?: boolean
  device?: DeviceItem
  mixer?: MixerState
  audioSources?: AudioSourceOption[]
  program?: ProgramState
  audioLoading?: boolean
  audioError?: string | null
}>()
const inputName = computed(() => props.device
  ? `${props.device.name}${props.device.input_type === 'v4l2' ? ` · ${props.device.input_value}` : ''}` : '设备输入')
const originalEnabled = computed(() => props.device?.include_audio === true)
const busId = computed(() => props.device?.config?.output?.audio_bus_id ??
  (props.device?.config?.output?.use_global_audio !== false && props.program?.config.enabled ? props.program.config.active_bus_id : null))
const bus = computed(() => props.mixer?.buses.find(item => item.id === busId.value))
const originalPresent = computed(() => inputStreams.value.some(stream => stream.kind === 'audio'))
const outputAudioPresent = computed(() => pipeline.value?.output.some(output => output.stream.kind === 'audio') ?? false)
const silentTrack = computed(() => !direct.value && !outputAudioPresent.value &&
  Boolean(distribution.value?.tracks.some(track => !track.is_video)))
const mixOriginal = computed(() => originalEnabled.value &&
  (props.device?.config?.output?.audio_bus_id ? props.device.config.output.audio_mode === 'add_original' : props.program?.config.mode === 'add_original'))
const audioOrigin = computed(() => {
  if (!props.device) return '音频来源待确认'
  if (direct.value) return '协议直接接入音频'
  if (props.audioLoading) return '音频来源加载中'
  if (props.audioError) return '音频来源待确认'
  if (busId.value && bus.value?.enabled && outputAudioPresent.value) {
    return `混音总线 ${busId.value}${mixOriginal.value ? ` + 输入原音频（${inputName.value}）` : ''}`
  }
  if (outputAudioPresent.value && originalEnabled.value && originalPresent.value) return `输入原音频（${inputName.value}）`
  if (outputAudioPresent.value) return '音频来源待确认'
  if (silentTrack.value) return '分发服务生成的静音音轨'
  return '暂无音频输出'
})
function sourceName(id: string): string {
  return props.audioSources?.find(source => source.id === id)?.name ?? id
}
function videoOrigin(): string {
  return `${inputName.value} → ${pipeline.value?.encoders.some(codec => codec.kind === 'video') ? '视频编码' : '视频直通'} → 视频输出`
}
const pipeline = computed(() => props.metrics?.pipeline)
const inputStreams = computed(() => pipeline.value?.input.filter(stream => stream.kind === 'video' || stream.kind === 'audio') ?? [])
const distribution = computed(() => props.metrics?.distribution)
const stateLabel = computed(() => props.stale ? '更新中断' : props.metrics?.state === 'running' ? '流处理中' : props.metrics?.state === 'starting' ? '连接中' : props.metrics ? '无输入' : '等待采样')
const direct = computed(() => !pipeline.value && Boolean(distribution.value))
function rate(bytes: number): string {
  const bits = Math.max(0, bytes) * 8
  if (bits >= 1_000_000_000) return `${(bits / 1_000_000_000).toFixed(2)} Gbps`
  if (bits >= 1_000_000) return `${(bits / 1_000_000).toFixed(2)} Mbps`
  if (bits >= 1000) return `${(bits / 1000).toFixed(1)} Kbps`
  return `${Math.round(bits)} bps`
}
function mediaDetail(stream: MediaStreamMetrics): string {
  if (stream.kind === 'video') return `${stream.width} × ${stream.height} · ${stream.fps?.toFixed(2).replace(/\.00$/, '') ?? '—'} FPS${stream.pixel_format ? ` · ${stream.pixel_format}` : ''}`
  if (stream.kind === 'audio') return `${stream.sample_rate / 1000} kHz · ${stream.channels} 声道`
  return stream.kind
}
function codecLabel(codec: CodecMetrics): string {
  return `${codec.hardware ? '硬件' : '软件'}${codec.running ? '' : ' · 已停止'}`
}
const inputRate = computed(() => pipeline.value ? inputStreams.value.reduce((sum, stream) => sum + stream.traffic.bytes_per_second, 0) : null)
const outputRate = computed(() => pipeline.value?.output.filter(output => output.traffic_available).reduce((sum, output) => sum + output.stream.traffic.bytes_per_second, 0) ?? distribution.value?.bytes_per_second ?? null)
</script>

<template>
  <section class="flow-topology" aria-label="设备媒体流拓扑" :class="{ 'flow-stale': stale }">
    <header class="flow-heading">
      <span class="flow-title"><i class="pi pi-share-alt" /> 媒体流拓扑</span>
      <span class="flow-summary">
        <span v-if="inputRate !== null">输入 {{ rate(inputRate) }}</span>
        <span v-if="outputRate !== null">输出 {{ rate(outputRate) }}</span>
        <Tag :value="stateLabel" :severity="stale ? 'warn' : metrics?.state === 'running' ? 'success' : 'secondary'" />
      </span>
    </header>
    <div v-if="device" class="flow-routes" aria-label="音视频来源">
      <div class="flow-route flow-video-route"><i class="pi pi-video" /><strong>视频</strong><span>{{ videoOrigin() }}</span></div>
      <div class="flow-route flow-audio-route"><i class="pi pi-volume-up" /><strong>输出音频</strong><span>{{ audioOrigin }}</span></div>
    </div>
    <div class="flow-chain" :class="{ 'flow-has-mixer': device && busId }">
      <div class="flow-node flow-input">
        <h3><i class="pi pi-sign-in" /> 输入 <span class="flow-input-format">{{ metrics?.input_type.toUpperCase() ?? '—' }}</span></h3>
        <div v-for="stream in inputStreams" :key="stream.index" class="flow-stream" :class="{ 'flow-disabled': device && stream.kind === 'audio' && !originalEnabled }">
          <strong>{{ stream.kind === 'audio' ? '音频' : '视频' }} · {{ stream.codec.toUpperCase() }}</strong>
          <span>{{ mediaDetail(stream) }}</span>
          <small>来源：{{ inputName }}{{ stream.kind === 'audio' ? ' · 输入原音频' : '' }}</small>
          <Tag v-if="stream.kind === 'audio' && device" :value="originalEnabled ? '原音频已启用' : '原音频已禁用 · 不进入输出'" :severity="originalEnabled ? 'success' : 'warn'" />
          <div class="flow-rate"><b>{{ rate(stream.traffic.bytes_per_second) }}</b><span>{{ stream.traffic.packets_per_second.toFixed(1) }} 包/s</span></div>
          <small v-if="stream.traffic.last_packet_age_ms !== null && stream.traffic.last_packet_age_ms >= 2000">{{ (stream.traffic.last_packet_age_ms / 1000).toFixed(0) }} 秒未收到数据</small>
        </div>
        <div v-if="device && !originalPresent" class="flow-stream flow-original" :class="{ 'flow-disabled': !originalEnabled }">
          <strong><i class="pi pi-volume-up" /> 输入原音频</strong>
          <Tag :value="originalEnabled ? '已启用 · 未检测到输入音轨' : '已禁用 · 不进入输出'" :severity="originalEnabled ? 'secondary' : 'warn'" />
          <span>来源：{{ inputName }}</span>
        </div>
        <p v-if="!inputStreams.length">{{ direct ? '协议直接接入，未经过 FFmpeg 采集' : '等待输入音视频数据' }}</p>
      </div>
      <span class="flow-arrow" aria-hidden="true">→</span>
      <div class="flow-node flow-codec">
        <h3><i class="pi pi-microchip" /> 解码</h3>
        <div v-for="codec in pipeline?.decoders ?? []" :key="`${codec.stream_index}-${codec.codec}`" class="flow-stream">
          <strong>{{ codec.codec }}</strong>
          <Tag :value="codecLabel(codec)" :severity="codec.hardware && codec.running ? 'success' : 'secondary'" />
          <span>{{ codec.kind === 'audio' ? '音频' : '视频' }} · 流 {{ codec.stream_index }}</span>
          <div class="flow-rate"><b>{{ codec.traffic.packets_per_second.toFixed(1) }} {{ codec.kind === 'video' ? 'FPS' : '帧/s' }}</b></div>
        </div>
        <p v-if="!pipeline?.decoders.length">{{ inputStreams.length || direct ? '无需解码' : '等待输入' }}</p>
      </div>
      <span class="flow-arrow" aria-hidden="true">→</span>
      <div class="flow-node flow-codec">
        <h3><i class="pi pi-microchip" /> 编码</h3>
        <div v-for="(codec, index) in pipeline?.encoders ?? []" :key="`${index}-${codec.stream_index}-${codec.codec}`" class="flow-stream">
          <strong>{{ codec.codec }}</strong>
          <Tag :value="codecLabel(codec)" :severity="codec.hardware && codec.running ? 'success' : 'secondary'" />
          <span>{{ codec.kind === 'audio' ? '音频' : '视频' }} · 流 {{ codec.stream_index }}</span>
          <div class="flow-rate"><b>{{ codec.traffic.packets_per_second.toFixed(1) }} {{ codec.kind === 'video' ? 'FPS' : '帧/s' }}</b><span>{{ rate(codec.traffic.bytes_per_second) }}</span></div>
        </div>
        <p v-if="!pipeline?.encoders.length">{{ inputStreams.length || direct ? '原始编码直通' : '等待输入' }}</p>
      </div>
      <span class="flow-arrow flow-video-arrow" aria-hidden="true">→</span>
      <div class="flow-node flow-output">
        <h3><i class="pi pi-sign-out" /> 输出</h3>
        <div v-for="output in pipeline?.output ?? []" :key="output.id" class="flow-stream">
          <strong>{{ output.stream.kind === 'audio' ? '音频' : '视频' }} · {{ output.stream.codec.toUpperCase() }}</strong>
          <span class="flow-origin" :class="output.stream.kind === 'audio' ? 'flow-audio-origin' : 'flow-video-origin'">来源：{{ output.stream.kind === 'audio' ? audioOrigin : inputName }}</span>
          <span>{{ mediaDetail(output.stream) }}</span>
          <small v-if="!output.traffic_available">此输出的码率暂无采样</small>
          <div v-if="output.traffic_available" class="flow-rate"><b>{{ rate(output.stream.traffic.bytes_per_second) }}</b><span>{{ output.stream.traffic.packets_per_second.toFixed(1) }} 包/s</span></div>
        </div>
        <template v-if="direct">
          <div v-for="(track, index) in distribution?.tracks ?? []" :key="index" class="flow-stream">
            <strong>{{ track.is_video ? '视频' : '音频' }} · {{ track.codec_name }}</strong>
            <span class="flow-origin">来源：{{ track.is_video ? inputName : audioOrigin }}</span>
            <span>{{ track.is_video ? `${track.width} × ${track.height} · ${track.fps} FPS` : `${track.sample_rate / 1000} kHz · ${track.channels} 声道` }}</span>
          </div>
        </template>
        <div v-if="silentTrack" class="flow-stream flow-disabled"><strong>音频 · 静音音轨</strong><span>来源：分发服务生成，无输入音频数据</span></div>
        <p v-if="!pipeline?.output.length && !direct">暂无输出流</p>
      </div>
    <div v-if="device && (busId || audioError)" class="flow-audio-path" aria-label="音频来源详情">
      <div v-if="busId" class="flow-mixer-path">
        <strong><i class="pi pi-volume-up" /> 混音总线 {{ busId }}</strong>
        <Tag :value="audioLoading ? '加载中' : audioError ? '状态获取失败' : bus?.enabled ? '已启动' : '未启动'" :severity="!audioLoading && !audioError && bus?.enabled ? 'success' : 'warn'" />
        <div v-for="input in bus?.inputs ?? []" :key="input.source_id" class="flow-route" :class="{ 'flow-disabled': input.muted || input.volume === 0 || !bus?.enabled }">
          <span>{{ sourceName(input.source_id) }}</span><span>{{ input.volume }}%{{ input.muted ? ' · 已静音' : '' }}</span>
          <span aria-hidden="true">→</span><span>混音总线 {{ busId }}</span><span aria-hidden="true">→</span><span>{{ !bus?.enabled ? '未进入输出' : input.muted || input.volume === 0 ? '静音 · 不贡献声音' : '输出音频' }}</span>
        </div>
        <span v-if="bus && !bus.enabled" class="flow-audio-note">总线未启动，当前输出未使用混音音频</span>
      </div>
      <span v-if="audioError" class="flow-audio-note">{{ audioError }}，请刷新音频来源</span>
    </div>
      <span v-if="device && busId" class="flow-arrow flow-mixer-arrow" :class="{ 'flow-connection-disabled': !bus?.enabled || audioError }" aria-label="混音总线连接到输出">→</span>
    </div>
    <footer class="flow-footer">
      <span>{{ metrics?.message ?? '每 2 秒刷新 · 码率按媒体数据计算' }}</span>
      <span v-if="distribution">{{ distribution.total_reader_count }} 个播放连接 · 分发 {{ rate(distribution.bytes_per_second) }}<template v-if="distribution.tracks.some(track => !track.is_video) && !pipeline?.output.some(output => output.stream.kind === 'audio') && !direct"> · 含静音音轨</template></span>
    </footer>
  </section>
</template>

<style scoped>
.flow-topology {
  --p-content-background: var(--p-surface-900);
  --p-content-border-color: var(--p-surface-700);
  --p-text-muted-color: var(--p-surface-400);

  color: var(--p-surface-100);
 width: 100%;
 max-width: 100%;
 box-sizing: border-box;
 container-type: inline-size;
 padding: 1rem;
 border: 1px solid var(--p-content-border-color);
 border-radius: 0.5rem;
 background: var(--p-content-background);

}

.flow-heading, .flow-summary, .flow-footer {
 display: flex;
 align-items: center;
 justify-content: space-between;
 gap: 1rem;
 flex-wrap: wrap;

}

.flow-title {
 display: flex;
 align-items: center;
 gap: 0.5rem;
 font-weight: 600;
 font-size: 0.875rem;

}

.flow-summary {
 font-size: 0.75rem;
 font-variant-numeric: tabular-nums;

}

.flow-chain {
 display: grid;
 grid-template-columns: minmax(0, 1.5fr) 1.5rem minmax(0, 1fr) 1.5rem minmax(0, 1fr) 1.5rem minmax(0, 1.5fr);
 align-items: stretch;
 margin-top: 0.875rem;
 gap: 0.25rem;

}

.flow-node {
 border: 1px solid var(--p-content-border-color);
 border-radius: 0.375rem;
 padding: 0.75rem;
 min-width: 0;

}

.flow-node h3 {
 display: flex;
 align-items: center;
 gap: 0.5rem;
 margin: 0 0 0.75rem;
 font-size: 0.75rem;
 font-weight: 600;
 color: var(--p-text-muted-color);

}

.flow-input-format {
 margin-left: auto;
 font-size: 0.6875rem;

}

.flow-input {
 border-top: 2px solid var(--p-primary-color);

}

.flow-output {
 border-top: 2px solid var(--p-green-400);

}

.flow-codec {
 border-top: 2px solid var(--p-content-border-color);

}

.flow-arrow {
 display: flex;
 align-items: center;
 justify-content: center;
 color: var(--p-text-muted-color);
 font-size: 1.25rem;

}

.flow-stream {
 display: flex;
 flex-direction: column;
 align-items: flex-start;
 gap: 0.375rem;
 font-size: 0.75rem;

}

.flow-stream + .flow-stream {
 margin-top: 0.75rem;
 padding-top: 0.75rem;
 border-top: 1px solid var(--p-content-border-color);

}

.flow-stream strong, .flow-rate b {
 font-family: ui-monospace, monospace;
 font-weight: 600;
 overflow-wrap: anywhere;

}

.flow-stream > span, .flow-stream small, .flow-footer {
 color: var(--p-text-muted-color);

}

.flow-stream small {
 color: var(--p-orange-400);

}

.flow-rate {
 display: flex;
 gap: 0.5rem;
 flex-wrap: wrap;
 font-variant-numeric: tabular-nums;

}

.flow-rate span {
 color: var(--p-text-muted-color);

}

.flow-node p {
 font-size: 0.75rem;
 color: var(--p-text-muted-color);
 margin: 0;

}

.flow-footer {
 margin-top: 0.75rem;
 font-size: 0.6875rem;

}

.flow-stale .flow-chain {
 opacity: 0.6;

}


.flow-routes, .flow-audio-path {
 display: grid;
 gap: 0.625rem;
 margin-top: 0.875rem;
}

.flow-route {
 display: flex;
 align-items: center;
 flex-wrap: wrap;
 gap: 0.5rem;
 font-size: 0.75rem;
 overflow-wrap: anywhere;
}

.flow-video-route, .flow-audio-route {
 border-left: 3px solid var(--p-primary-color);
 padding-left: 0.625rem;
}

.flow-audio-route {
 border-color: var(--p-green-400);
}

.flow-audio-path {
 padding: 0.75rem;
 border: 1px solid var(--p-content-border-color);
 border-radius: 0.375rem;
}

.flow-mixer-path {
 display: flex;
 align-items: center;
 flex-wrap: wrap;
 gap: 0.5rem;
 font-size: 0.75rem;
}

.flow-mixer-path .flow-route {
 flex-basis: 100%;
}

.flow-disabled {
 border: 1px dashed var(--p-surface-500);
 border-radius: 0.25rem;
 padding: 0.5rem;
 color: var(--p-surface-400);
}

.flow-stream .flow-video-origin {
 color: var(--p-primary-color);
}

.flow-stream .flow-audio-origin {
 color: var(--p-green-400);
}

.flow-audio-note {
 color: var(--p-orange-400);
 font-size: 0.75rem;
}

.flow-has-mixer .flow-output {
 grid-column: 7;
 grid-row: 1 / span 2;
}

.flow-chain .flow-audio-path {
 grid-column: 1 / span 5;
 grid-row: 2;
}

.flow-mixer-arrow {
 grid-column: 6;
 grid-row: 2;
 color: var(--p-green-400);
 font-size: 1.75rem;
}

.flow-connection-disabled {
 color: var(--p-surface-500);
}

@container (width <= 900px) {
 .flow-chain {
 grid-template-columns: 1fr;

}

 .flow-has-mixer .flow-output, .flow-chain .flow-audio-path, .flow-mixer-arrow {
 grid-column: 1;
 grid-row: auto;
}

 .flow-has-mixer .flow-output {
 order: 3;
}

 .flow-chain .flow-audio-path {
 order: 1;
}

 .flow-mixer-arrow {
 order: 2;
}

 .flow-has-mixer .flow-video-arrow {
 display: none;
}

 .flow-arrow {
 transform: rotate(90deg);
 padding: 0.125rem;

}

}

</style>
