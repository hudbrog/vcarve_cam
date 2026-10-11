"""Fetch immutable inputs for the isolated experiment; production stays untouched."""
from pathlib import Path
import hashlib
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2] / 'artifacts/sketcher-feasibility/upstream'
INPUTS = [
    ('arael.zip', 'https://codeload.github.com/harakas/arael/zip/18d86e1fda91814a4a42d00001f3e8461c638bb4', 'a81d9a9621ad643981a5fe481be8d1c672d202471bf29f108f6fc6d760f9e03d', 'arael-18d86e1fda91814a4a42d00001f3e8461c638bb4'),
    ('planegcs.zip', 'https://codeload.github.com/Salusoft89/planegcs/zip/9d26baef0b240fe0eef4186d4e5e5e66ec3620b7', 'cf7d4e27fd13fd169956cfecd60e10df2551b545921e4fa654f3d51b26fc0363', 'planegcs-9d26baef0b240fe0eef4186d4e5e5e66ec3620b7'),
    ('planegcs-1.3.0.tgz', 'https://registry.npmjs.org/@salusoft89/planegcs/-/planegcs-1.3.0.tgz', '8bd29ab7e1ba2c3e370431289d369087078e803bb59ecbf3732374ea98a12b20', 'plane-package'),
    ('eigen.zip', 'https://gitlab.com/libeigen/eigen/-/archive/3.4.0/eigen-3.4.0.zip', 'eba3f3d414d2f8cba2919c78ec6daab08fc71ba2ba4ae502b7e5d4d99fc02cda', 'eigen-3.4.0'),
    ('boost.tar.gz', 'https://archives.boost.io/release/1.86.0/source/boost_1_86_0.tar.gz', '2575e74ffc3ef1cd0babac2c1ee8bdb5782a0ee672b1912da40e5b4b591ca01f', 'boost_1_86_0'),
    ('json.hpp', 'https://raw.githubusercontent.com/nlohmann/json/v3.12.0/single_include/nlohmann/json.hpp', 'aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63', None),
]

def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    for name, url, digest, directory in INPUTS:
        archive = ROOT / name
        if not archive.exists():
            print('Downloading', name, flush=True)
            with urllib.request.urlopen(url, timeout=180) as response, archive.open('wb') as out:
                while chunk := response.read(1024 * 1024):
                    out.write(chunk)
        actual = hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()
        if actual != digest:
            raise ValueError(f'{name}: SHA-256 mismatch; no extraction performed')
        if directory and not (ROOT / directory).exists():
            print('Extracting', name, flush=True)
            if name.endswith('.zip'):
                with zipfile.ZipFile(archive) as z:
                    for entry in z.infolist():
                        if not (ROOT / entry.filename).resolve().is_relative_to(ROOT):
                            raise ValueError('Unsafe archive path')
                    z.extractall(ROOT)
            else:
                destination = ROOT / directory if name.endswith('.tgz') else ROOT
                destination.mkdir(parents=True, exist_ok=True)
                with tarfile.open(archive) as t:
                    members = [m for m in t if m.isfile() and (name.endswith('.tgz') or m.name.startswith('boost_1_86_0/boost/') or m.name == 'boost_1_86_0/LICENSE_1_0.txt')]
                    t.extractall(destination, members=members, filter='data')
        print('Verified', name, flush=True)

if __name__ == '__main__':
    main()
