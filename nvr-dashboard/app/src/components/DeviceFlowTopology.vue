<script setup lang="ts">
import { computed } from 'vue'
import Tag from 'primevue/tag'
import type { CodecMetrics, DeviceMetrics, MediaStreamMetrics } from '../api/device'

const props = defineProps<{ metrics?: DeviceMetrics; stale?: boolean }>()
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
    <div class="flow-chain">
      <div class="flow-node flow-input">
        <h3><i class="pi pi-sign-in" /> 输入 <span class="flow-input-format">{{ metrics?.input_type.toUpperCase() ?? '—' }}</span></h3>
        <div v-for="stream in inputStreams" :key="stream.index" class="flow-stream">
          <strong>{{ stream.kind === 'audio' ? '音频' : '视频' }} · {{ stream.codec.toUpperCase() }}</strong>
          <span>{{ mediaDetail(stream) }}</span>
          <div class="flow-rate"><b>{{ rate(stream.traffic.bytes_per_second) }}</b><span>{{ stream.traffic.packets_per_second.toFixed(1) }} 包/s</span></div>
          <small v-if="stream.traffic.last_packet_age_ms !== null && stream.traffic.last_packet_age_ms >= 2000">{{ (stream.traffic.last_packet_age_ms / 1000).toFixed(0) }} 秒未收到数据</small>
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
      <span class="flow-arrow" aria-hidden="true">→</span>
      <div class="flow-node flow-output">
        <h3><i class="pi pi-sign-out" /> 输出</h3>
        <div v-for="output in pipeline?.output ?? []" :key="output.id" class="flow-stream">
          <strong>{{ output.stream.kind === 'audio' ? '音频' : '视频' }} · {{ output.stream.codec.toUpperCase() }}</strong>
          <span>{{ mediaDetail(output.stream) }}</span>
          <small v-if="!output.traffic_available">此输出的码率暂无采样</small>
          <div v-if="output.traffic_available" class="flow-rate"><b>{{ rate(output.stream.traffic.bytes_per_second) }}</b><span>{{ output.stream.traffic.packets_per_second.toFixed(1) }} 包/s</span></div>
        </div>
        <template v-if="direct">
          <div v-for="(track, index) in distribution?.tracks ?? []" :key="index" class="flow-stream">
            <strong>{{ track.is_video ? '视频' : '音频' }} · {{ track.codec_name }}</strong>
            <span>{{ track.is_video ? `${track.width} × ${track.height} · ${track.fps} FPS` : `${track.sample_rate / 1000} kHz · ${track.channels} 声道` }}</span>
          </div>
        </template>
        <p v-if="!pipeline?.output.length && !direct">暂无输出流</p>
      </div>
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


@container (width <= 900px) {
 .flow-chain {
 grid-template-columns: 1fr;

}

 .flow-arrow {
 transform: rotate(90deg);
 padding: 0.125rem;

}

}

</style>
