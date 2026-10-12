"""Exercise the real built dashboard and actual recorded TS segments."""

import json
import re

from suite import Skip


def verify_playback(s, device, segments):
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as error:
        raise Skip(
            'Python Playwright is missing; real recording verified, browser continuation untested'
        ) from error
    with sync_playwright() as p:
        try:
            browser = p.chromium.launch(
                headless=True, args=['--autoplay-policy=no-user-gesture-required']
            )
        except Exception as error:
            raise Skip('Chromium cannot launch: ' + str(error).splitlines()[0]) from error
        try:
            page = browser.new_page()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.add_init_script(
                'sessionStorage.setItem("nvr-auth-token-session", ' + json.dumps(s.token) + ')'
            )
            page.goto(s.base + '/nvr/playback')
            page.wait_for_load_state('networkidle')
            # Keep DOM evidence so a changed UI selector is diagnosable.
            (s.artifacts / 'playback.html').write_text(s.redact(page.content()))
            page.get_by_role('tab').filter(has_text=device['name']).click()
            page.locator('.segment-card').filter(has_text=segments[0]['file_name']).get_by_role(
                'button', name='播放', exact=True
            ).click()
            video = page.locator('video').first
            video.wait_for(state='visible')
            page.wait_for_function(
                """() => {
                const v = document.querySelector('video');
                return v && v.readyState >= 2 && v.currentTime > 0 && !v.paused;
            }""",
                timeout=30000,
            )
            # Seek close to the real end; do not synthesize an ended event.
            assert video.evaluate('(v) => v.currentSrc.startsWith("blob:")'), (
                'Chromium did not use MSE/hls.js'
            )
            page.wait_for_function(
                """name => [...document.querySelectorAll('.segment-card-active .segment-file-name')].some(e => e.textContent === name)""",
                arg=segments[0]['file_name'],
            )
            playlists = []
            page.on(
                'request',
                lambda request: (
                    playlists.append(request.url) if '/segment-playlist/' in request.url else None
                ),
            )
            page.evaluate("""() => {
                const v = document.querySelector('video');
                if (!Number.isFinite(v.duration) || v.duration < 2) throw new Error('invalid segment duration');
                v.currentTime = v.duration - 1.5;
            }""")
            page.wait_for_function(
                """() => {
                const v = document.querySelector('video');
                return v && v.currentTime < 10 && v.currentTime > 0.1 && !v.paused && v.readyState >= 2;
            }""",
                timeout=30000,
            )
            assert any('/segment-playlist/' + segments[1]['id'] in url for url in playlists), (
                'No next segment playlist request after actual end'
            )
            before = video.evaluate('(v) => v.currentTime')
            page.wait_for_timeout(1500)
            after = video.evaluate('(v) => v.currentTime')
            assert after > before + 0.5, f'next segment stalled: {before} -> {after}'
            page.wait_for_function(
                """name => [...document.querySelectorAll('.segment-card-active .segment-file-name')].some(e => e.textContent === name)""",
                arg=segments[1]['file_name'],
            )
            assert not errors, errors
            # Daily playlist exposes the index badge; segment-card mode does not.
            page.reload()
            page.wait_for_load_state('networkidle')
            page.get_by_role('tab').filter(has_text=device['name']).click()
            page.get_by_role('button', name='打开回放', exact=True).click()
            page.wait_for_function(
                "() => { const v = document.querySelector('video'); return v && v.readyState >= 2 && v.currentTime > 0 && !v.paused; }",
                timeout=30000,
            )
            page.get_by_text(re.compile(r'^1 / [1-9][0-9]*$'), exact=True).wait_for(state='visible')
            page.evaluate(
                '(end) => { document.querySelector("video").currentTime = end - 1.5; }',
                segments[0]['duration'],
            )
            page.wait_for_function(
                '(end) => { const v = document.querySelector("video"); return v && v.currentTime > end + 0.3 && !v.paused && v.readyState >= 2; }',
                arg=segments[0]['duration'],
                timeout=30000,
            )
            page.get_by_text(re.compile(r'^2 / [1-9][0-9]*$'), exact=True).wait_for(state='visible')
            assert not errors, errors
            page.screenshot(path=str(s.artifacts / 'playback.png'), full_page=True)
            s.last = f'Browser crossed real segment boundary; time {before:.2f}->{after:.2f}'
        except Exception:
            page.screenshot(path=str(s.artifacts / 'playback-failure.png'), full_page=True)
            (s.artifacts / 'playback-failure.html').write_text(s.redact(page.content()))
            raise
        finally:
            browser.close()
