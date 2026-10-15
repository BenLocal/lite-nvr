import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DeviceFlowTopology from './DeviceFlowTopology.vue'
import type { MixerState } from '../api/audiomixer'
import { parseDeviceMetrics, type DeviceMetrics, type DeviceItem } from '../api/device'

const sample: DeviceMetrics = {
  id: 'capture', input_type: 'v4l2', state: 'running', message: null, distribution: null, sampled_at_ms: 123,
  pipeline: {
    running: true,
    input: [{ index: 0, kind: 'video', codec: 'rawvideo', width: 1920, height: 1080, fps: 60, pixel_format: 'nv12', sample_rate: 0, channels: 0,
      traffic: { bytes_total: 6000, packets_total: 60, bytes_per_second: 3_000_000, packets_per_second: 60, last_packet_age_ms: 0 } }],
    decoders: [],
    encoders: [{ stream_index: 0, kind: 'video', codec: 'h264_rkmpp', hardware: true, running: true, traffic: { bytes_total: 2000, packets_total: 60, bytes_per_second: 125_000, packets_per_second: 60, last_packet_age_ms: 0 } }],
    output: [{ id: 'video', destination: 'encoded', traffic_available: true, stream: { index: 0, kind: 'video', codec: 'h264', width: 1920, height: 1080, fps: 60, pixel_format: 'nv12', sample_rate: 0, channels: 0,
      traffic: { bytes_total: 3000, packets_total: 60, bytes_per_second: 125_000, packets_per_second: 60, last_packet_age_ms: 0 } } }],
  },
}

describe('device flow topology', () => {
  it('shows actual hardware selection and switches to software after fallback', async () => {
    const wrapper = mount(DeviceFlowTopology, { props: { metrics: sample } })
    expect(wrapper.text()).toContain('24.00 Mbps')
    expect(wrapper.text()).toContain('1.00 Mbps')
    expect(wrapper.text()).toContain('1920 × 1080 · 60 FPS · nv12')
    expect(wrapper.text()).toContain('硬件')
    expect(wrapper.text()).toContain('60.0 FPS')
    expect(wrapper.text()).toContain('无需解码')
    const software = structuredClone(sample)
    if (!software.pipeline) throw new Error('fixture pipeline missing')
    software.pipeline.encoders[0] = { stream_index: 0, kind: 'video', codec: 'libx264', hardware: false, running: true, traffic: { bytes_total: 2000, packets_total: 60, bytes_per_second: 125_000, packets_per_second: 60, last_packet_age_ms: 0 } }
    await wrapper.setProps({ metrics: software, stale: true })
    expect(wrapper.text()).toContain('软件')
    expect(wrapper.text()).toContain('libx264')
    expect(wrapper.text()).not.toContain('硬件')
    expect(wrapper.text()).toContain('更新中断')
  })
  it('marks a decoder that decodes keyframes only', () => {
    const keyOnly = structuredClone(sample)
    if (!keyOnly.pipeline) throw new Error('fixture pipeline missing')
    keyOnly.pipeline.decoders = [{ stream_index: 0, kind: 'video', codec: 'h264_rkmpp', hardware: true, running: true, key_frames_only: true, traffic: { bytes_total: 0, packets_total: 2, bytes_per_second: 0, packets_per_second: 0.5, last_packet_age_ms: 0 } }]
    const wrapper = mount(DeviceFlowTopology, { props: { metrics: keyOnly } })
    expect(wrapper.text()).toContain('硬件 · 仅关键帧')
    expect(parseDeviceMetrics([keyOnly])[0]?.pipeline?.decoders[0]?.key_frames_only).toBe(true)
  })
  it('omits RTMP metadata streams from audio/video inputs', () => {
    const metadata = structuredClone(sample)
    if (!metadata.pipeline) throw new Error('fixture pipeline missing')
    const video = metadata.pipeline.input[0]
    if (!video) throw new Error('fixture video missing')
    metadata.pipeline.input.unshift({ ...video, index: 8, kind: 'other', codec: 'none' })
    const wrapper = mount(DeviceFlowTopology, { props: { metrics: metadata } })
    expect(wrapper.text()).not.toContain('NONE')
    expect(wrapper.text()).not.toContain('other')
    expect(wrapper.text()).toContain('RAWVIDEO')
  })
  it('keeps a direct protocol source distinct from FFmpeg telemetry', () => {
    const direct: DeviceMetrics = { ...sample, input_type: 'gb28181', pipeline: null, distribution: {
      schema: 'rtsp', app: 'rtp', stream: 'capture', bytes_per_second: 125000, total_reader_count: 2,
      tracks: [{ codec_name: 'H264', is_video: true, width: 1280, height: 720, fps: 25, sample_rate: 0, channels: 0, bit_rate: 1000000 }],
    } }
    const wrapper = mount(DeviceFlowTopology, { props: { metrics: direct } })
    expect(wrapper.text()).toContain('协议直接接入')
    expect(wrapper.text()).toContain('1280 × 720 · 25 FPS')
    expect(wrapper.text()).toContain('2 个播放连接')
    expect(wrapper.text()).not.toContain('硬件')
  })
  it('validates metrics before rendering invalid rates or codec flags', () => {
    expect(parseDeviceMetrics([sample])).toHaveLength(1)
    const invalid = structuredClone(sample)
    if (!invalid.pipeline) throw new Error('fixture pipeline missing')
    invalid.pipeline.input[0]!.traffic.bytes_per_second = Infinity
    expect(() => parseDeviceMetrics([invalid])).toThrow('格式不正确')
    expect(() => parseDeviceMetrics([{ ...sample, state: ['running'] }])).toThrow('格式不正确')
    expect(() => parseDeviceMetrics([{ ...sample, pipeline: { ...sample.pipeline, encoders: [{ hardware: 'true' }] } }])).toThrow('格式不正确')
  })
})

const device: DeviceItem = {
  id: 'capture', name: 'HDMI 1', input_type: 'v4l2', input_value: '/dev/video0', description: '',
  include_audio: false, record: false, created_at: '', updated_at: '',
  config: { output: { audio_bus_id: 'first', audio_mode: 'program' } },
}
const mixer: MixerState = {
  sources: [], buses: [{ id: 'first', enabled: true, publish_url: '', flv_url: '',
    profile: { codec: 'aac', sample_rate: 48000, channels: 2, bitrate: 128000 },
    inputs: [{ source_id: 'alsa:D6911codec', volume: 165, muted: false }, { source_id: 'alsa:rockchiphdmiin', volume: 174, muted: true }],
  }],
}
function withAudio(): DeviceMetrics {
  const metrics = structuredClone(sample)
  if (!metrics.pipeline) throw new Error('fixture pipeline missing')
  const video = metrics.pipeline.input[0]
  if (!video) throw new Error('fixture video missing')
  const audio = { ...video, index: 1, kind: 'audio', codec: 'aac', sample_rate: 48000, channels: 2 }
  metrics.pipeline.input.push(audio)
  metrics.pipeline.output.push({ id: 'audio', destination: 'encoded', traffic_available: true, stream: audio })
  return metrics
}

describe('audio and video provenance', () => {
  it('shows disabled original audio even when an input audio track exists, and traces hardware mixing', () => {
    const wrapper = mount(DeviceFlowTopology, { props: { device, metrics: withAudio(), mixer,
      audioSources: [{ id: 'alsa:D6911codec', name: 'HDMI 1 音频', kind: 'alsa' }, { id: 'alsa:rockchiphdmiin', name: 'HDMI 2 音频', kind: 'alsa' }],
    } })
    expect(wrapper.find('.flow-input').text()).toContain('原音频已禁用 · 不进入输出')
    expect(wrapper.find('.flow-output').text()).toContain('来源：混音总线 first')
    expect(wrapper.find('.flow-video-route').text()).toContain('HDMI 1 · /dev/video0 → 视频编码 → 视频输出')
    expect(wrapper.find('.flow-mixer-path').text()).toContain('HDMI 1 音频165%')
    expect(wrapper.find('.flow-mixer-path').text()).toContain('HDMI 2 音频174% · 已静音')
    expect(wrapper.find('.flow-mixer-path .flow-disabled').exists()).toBe(true)
  })
  it('retains a disabled original-audio node when the input has no audio track', () => {
    const wrapper = mount(DeviceFlowTopology, { props: { device, metrics: sample, mixer } })
    expect(wrapper.find('.flow-original').text()).toContain('已禁用 · 不进入输出')
    expect(wrapper.find('.flow-audio-route').text()).toContain('暂无音频输出')
  })
  it('identifies input audio without mixing and original-audio addition with mixing', async () => {
    const wrapper = mount(DeviceFlowTopology, { props: { device: { ...device, include_audio: true, config: { output: { use_global_audio: false } } }, metrics: withAudio(), mixer } })
    expect(wrapper.find('.flow-output').text()).toContain('输入原音频（HDMI 1 · /dev/video0）')
    await wrapper.setProps({ device: { ...device, include_audio: true, config: { output: { audio_bus_id: 'first', audio_mode: 'add_original' } } } })
    expect(wrapper.find('.flow-output').text()).toContain('混音总线 first + 输入原音频')
  })
  it('does not attribute silent audio or unknown audio to a stopped mixer', () => {
    const metrics = structuredClone(sample)
    metrics.distribution = { schema: 'rtsp', app: 'device', stream: 'capture', total_reader_count: 0, bytes_per_second: 1,
      tracks: [{ codec_name: 'AAC', is_video: false, width: 0, height: 0, fps: 0, sample_rate: 8000, channels: 1, bit_rate: 0 }],
    }
    const stopped = { ...mixer, buses: mixer.buses.map(bus => ({ ...bus, enabled: false })) }
    const wrapper = mount(DeviceFlowTopology, { props: { device, metrics, mixer: stopped } })
    expect(wrapper.find('.flow-output').text()).toContain('来源：分发服务生成，无输入音频数据')
    expect(wrapper.find('.flow-audio-route').text()).toContain('分发服务生成的静音音轨')
    const unknown = mount(DeviceFlowTopology, { props: { device, metrics: withAudio(), mixer: stopped, audioError: '连接失败' } })
    expect(unknown.find('.flow-output').text()).toContain('音频来源待确认')
    expect(unknown.text()).toContain('连接失败，请刷新音频来源')
  })
})
