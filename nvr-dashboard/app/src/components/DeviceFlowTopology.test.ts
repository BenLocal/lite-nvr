import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DeviceFlowTopology from './DeviceFlowTopology.vue'
import { parseDeviceMetrics, type DeviceMetrics } from '../api/device'

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
