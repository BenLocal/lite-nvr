"""Physical wiring must be explicit, and readiness must observe the pattern."""

import os
import shutil
import subprocess
import unittest
from unittest.mock import patch

from hdmi import COLOR_BARS, RGB_SAMPLE_FILTER, matches_color_bars, output_config
from suite import Skip


class WiringTests(unittest.TestCase):
    def test_missing_wiring_does_not_choose_a_default_card_or_connector(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(Skip):
                output_config()

    def test_selected_wiring_is_used_without_a_port_mapping(self):
        with patch.dict(
            os.environ,
            {'RK_DRM_CARD': '/dev/dri/card7', 'RK_HDMI_CONNECTOR': 'HDMI-A-4'},
            clear=True,
        ):
            self.assertEqual(output_config(), ('/dev/dri/card7', 'HDMI-A-4'))

    def test_pattern_readiness_accepts_captured_color_bars(self):
        self.assertTrue(matches_color_bars([channel for rgb in COLOR_BARS for channel in rgb]))

    def test_pattern_readiness_rejects_an_unrelated_white_signal(self):
        self.assertFalse(matches_color_bars([255] * 24))
        self.assertFalse(matches_color_bars([255] * 23))

    @unittest.skipUnless(shutil.which('ffmpeg'), 'FFmpeg is required for pixel sampling')
    def test_yuv420_sampling_converts_to_rgb_before_one_row_downscale(self):
        width, height = 1920, 1080
        row = bytes(channel for rgb in COLOR_BARS for _ in range(width // 8) for channel in rgb)
        frame = row * (height // 2) + bytes(width * 3 * (height // 2))
        # Reproduce decoded H264's subsampled YUV layout without a codec dependency.
        yuv = subprocess.run(
            [
                'ffmpeg',
                '-v',
                'error',
                '-f',
                'rawvideo',
                '-pix_fmt',
                'rgb24',
                '-s',
                f'{width}x{height}',
                '-i',
                '-',
                '-frames:v',
                '1',
                '-pix_fmt',
                'yuv420p',
                '-f',
                'rawvideo',
                '-',
            ],
            input=frame,
            capture_output=True,
            check=True,
            timeout=20,
        ).stdout
        pixels = subprocess.run(
            [
                'ffmpeg',
                '-v',
                'error',
                '-f',
                'rawvideo',
                '-pix_fmt',
                'yuv420p',
                '-s',
                f'{width}x{height}',
                '-i',
                '-',
                '-frames:v',
                '1',
                '-vf',
                RGB_SAMPLE_FILTER,
                '-f',
                'rawvideo',
                '-',
            ],
            input=yuv,
            capture_output=True,
            check=True,
            timeout=20,
        ).stdout
        self.assertTrue(matches_color_bars(pixels), list(pixels))


if __name__ == '__main__':
    unittest.main()
