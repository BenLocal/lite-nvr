"""Source mutants are temporary and require paired real-board evidence."""

import json
import os

CASES = {
    'drop-state-field': ('handler/device.rs', 'T1.2', 'T1'),
    'drop-output-normalize': ('handler/device.rs', 'T1.3', 'T1'),
    'drop-raw-encode': ('init/device.rs', 'T2.1', 'T2'),
    'drop-x11-guard': ('handler/device.rs', 'T0.8', 'T0'),
    'drop-rollback': ('handler/device.rs', 'T1.9', 'T1'),
}


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('Mutation anchor is not unique; review against the current source')
    return text.replace(old, new)


def mutate(text, name):
    if name == 'drop-state-field':
        old = "    state: &'static str,\n"
        if text.count(old) != 2:
            raise ValueError('Device state mutation anchor changed')
        return text.replace(old, '    #[serde(skip_serializing)]\n' + old, 1)
    if name == 'drop-output-normalize':
        return replace_once(text, '.filter(|output| output != &Default::default())', '')
    if name == 'drop-rollback':
        return replace_once(
            text,
            '                    nvr_db::device::upsert(old, conn).await?;',
            '                    // mutation: skip persisted configuration restoration',
        )
    if name == 'drop-raw-encode':
        old = """    if matches!(device.input_type.as_str(), "v4l2" | "x11grab" | "lavfi") {
        outputs[0].encode = Some(EncodeConfig {
            pixel_format: Some("yuv420p".to_string()),
            ..EncodeConfig::default()
        });
    }"""
        return replace_once(text, old, '')
    old = '    crate::x11::validate_input(&device.input_type, &device.input_value).await?;'
    if text.count(old) != 2:
        raise ValueError('X11 mutation anchor changed')
    text = text.replace(old, '')
    return replace_once(
        text,
        '        ensure_device_pipe(detect_hub, device).await?;',
        '        if device.input_type != "x11grab" { ensure_device_pipe(detect_hub, device).await?; }',
    )


def run_mutation(runner, name, run_suite):
    source, target, group = CASES[name]
    if name == 'drop-rollback':
        runner.backup_source(runner.repo / 'nvr/src' / source)
        text = runner.mutation_file.read_text()
        needle = '    let applied: anyhow::Result<()> = async {'
        runner.change_source(
            replace_once(
                text,
                needle,
                needle
                + '\n        if device.description == "e2e-injected-apply-failure" { anyhow::bail!("e2e injected apply failure"); }',
            )
        )
        os.environ['RK_FAULT_APPLICATION'] = '1'
    runner.artifacts = runner.parent_artifacts / 'baseline'
    runner.artifacts.mkdir()
    runner.build()
    runner.deploy()
    runner.connect()
    runner.phase = 'mutation.baseline'
    if run_suite(group) != 0:
        raise RuntimeError('Unmutated baseline failed; mutation verification aborted')
    runner.check_suite_unchanged()
    baseline = json.loads((runner.artifacts / 'report.json').read_text())
    if not any(r['id'] == target and r['status'] == 'PASS' for r in baseline['results']):
        raise RuntimeError('Mutation target was not PASS in baseline')
    runner.stop()
    if runner.mutation_file is None:
        runner.backup_source(runner.repo / 'nvr/src' / source)
    runner.change_source(mutate(runner.mutation_file.read_text(), name))
    runner.artifacts = runner.parent_artifacts / 'mutant'
    runner.artifacts.mkdir()
    runner.run_id += '-mutant'
    runner.build()
    runner.deploy()
    runner.connect()
    runner.phase = 'mutation.verify'
    if run_suite(group) != 1:
        raise RuntimeError('Mutant did not produce an assertion failure')
    runner.check_suite_unchanged()
    report = json.loads((runner.artifacts / 'report.json').read_text())
    if not any(r['id'] == target and r['status'] == 'FAIL' for r in report['results']):
        raise RuntimeError('Expected mutation assertion did not fail')
    if any(
        r['status'] != 'PASS'
        for r in report['results']
        if r['id'].startswith('T0.') and r['id'] != target
    ):
        raise RuntimeError('Mutant deployment baseline was not healthy')
    allowed = {target}
    if name == 'drop-raw-encode':
        allowed |= {'T2.5', 'T2.6', 'T2.7', 'T2.8'}
    if any(r['id'] not in allowed for r in report['results'] if r['status'] == 'FAIL'):
        raise RuntimeError('Unexpected mutant failures require investigation')
    runner.restore_source()
    (runner.parent_artifacts / 'mutation.json').write_text(
        json.dumps(
            dict(mutation=name, detected=True, target=target, commit=runner.commit), indent=2
        )
    )
    print(f'# Mutation {name} detected by {target}; source restored', flush=True)
