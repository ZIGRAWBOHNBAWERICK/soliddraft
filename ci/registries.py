"""Read the CI registries from a checkout or a particular Git tree."""
import json
import pathlib
import re
import subprocess
import tomllib


class RegistryError(ValueError):
    """Fragments do not describe an unambiguous registry."""


def fragments(root, directory, suffix, revision=None):
    if revision is None:
        return [(path.relative_to(root).as_posix(), path.read_text())
                for path in sorted((root / directory).glob(f'*{suffix}')) if path.is_file()]
    command = ['git', '-C', str(root)]
    paths = subprocess.check_output(
        [*command, 'ls-tree', '-r', '-z', '--name-only', revision, '--', directory + '/']).decode().split('\0')
    pattern = re.compile(re.escape(directory) + r'/[^/]+' + re.escape(suffix))
    return [(path, subprocess.check_output([*command, 'show', f'{revision}:{path}']).decode())
            for path in sorted(paths) if pattern.fullmatch(path)]


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise RegistryError(f'duplicate key {key}')
        result[key] = value
    return result


def check_consumers(root, revision=None):
    result = {'format': 'soliddraft-check-consumers/1', 'files': {}, 'patterns': {}}
    entries = fragments(root, 'ci/check-consumers', '.json', revision)
    if not entries:
        raise RegistryError('ci/check-consumers: no JSON policy fragments')
    for path, text in entries:
        try:
            part = json.loads(text, object_pairs_hook=unique_object)
            if (not isinstance(part, dict) or not part or part.keys() - {'files', 'patterns'}
                    or not all(isinstance(table, dict) for table in part.values())):
                raise RegistryError('invalid schema')
            for kind, table in part.items():
                for key, value in table.items():
                    if key in result[kind]:
                        raise RegistryError(f'duplicate {kind} entry {key}')
                    result[kind][key] = value
        except ValueError as error:
            raise RegistryError(f'{path}: invalid JSON policy fragment: {error}') from error
    return result


def scenario_inventory(root, revision=None):
    def entries(directory):
        for path, text in fragments(root, directory, '.json', revision):
            entry = json.loads(text, object_pairs_hook=unique_object)
            if not isinstance(entry, dict) or entry.get('id') != pathlib.PurePosixPath(path).stem:
                raise RegistryError(f'{path}: scenario id must match the filename')
            yield entry

    scenarios = list(entries('ci/scenario-inventory'))
    if not scenarios:
        raise RegistryError('ci/scenario-inventory: no application scenarios')
    return {'format': 'soliddraft-scenario-inventory/1', 'scenarios': scenarios,
            'native_ui_scenarios': list(entries('ci/scenario-inventory/native-ui'))}


def ownership(root):
    entries = fragments(root, 'ci/ownership', '.toml')
    if not entries:
        raise RegistryError('ci/ownership: no ownership fragments')
    areas = []
    names = set()
    for path, text in entries:
        for area in tomllib.loads(text)['areas']:
            name = area['name']
            if name in names:
                raise RegistryError(f'{path}: duplicate area {name}')
            names.add(name)
            areas.append(area)
    return areas
