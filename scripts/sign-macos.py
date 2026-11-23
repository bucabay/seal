#!/usr/bin/python3
"""Provision and sign Seal/Keymaker bundles with per-item keychain access."""
import argparse
import datetime
import fnmatch
import hashlib
import pathlib
import plistlib
import re
import shutil
import ssl
import subprocess
import tempfile


def validate_profile(profile, team, identifier, certificate_hash):
    if "OSX" not in profile.get("Platform", []):
        raise ValueError("this build requires a macOS provisioning profile; an iOS profile cannot authorize it")
    entitlements = profile.get("Entitlements", {})
    if entitlements.get("com.apple.developer.team-identifier") != team:
        raise ValueError("provisioning profile does not authorize this signing team")
    app_id = entitlements.get("com.apple.application-identifier", "")
    if not fnmatch.fnmatchcase(f"{team}.{identifier}", app_id):
        raise ValueError(f"provisioning profile does not authorize bundle ID {identifier}")
    group = f"{team}.app.seal.secure.secrets"
    if not any(fnmatch.fnmatchcase(group, allowed)
               for allowed in entitlements.get("keychain-access-groups", [])):
        raise ValueError(f"provisioning profile does not authorize keychain group {group}")
    expiry = profile.get("ExpirationDate")
    if expiry is None or expiry.replace(tzinfo=datetime.timezone.utc) <= datetime.datetime.now(datetime.timezone.utc):
        raise ValueError("provisioning profile is expired or has no expiration date")
    certificates = {hashlib.sha1(cert).hexdigest().upper()
                    for cert in profile.get("DeveloperCertificates", [])}
    if certificate_hash.upper() not in certificates:
        raise ValueError("provisioning profile does not authorize the selected signing certificate")
    return {
        "com.apple.application-identifier": f"{team}.{identifier}",
        "com.apple.developer.team-identifier": team,
        "keychain-access-groups": [group],
    }


def certificate_hash(identity):
    if re.fullmatch(r"[A-Fa-f0-9]{40}", identity):
        return identity.upper()
    pem = subprocess.check_output(["security", "find-certificate", "-c", identity, "-p"], text=True)
    return hashlib.sha1(ssl.PEM_cert_to_DER_cert(pem)).hexdigest().upper()


def cli_bundle(source, identifier):
    # A raw Mach-O cannot carry a provisioning profile. Execute this copy from
    # its app-like bundle; signing the original raw binary would not suffice.
    bundle = source.with_name(source.name + ".app")
    contents = bundle / "Contents"
    info_path = contents / "Info.plist"
    if bundle.exists():
        if not info_path.exists() or plistlib.loads(info_path.read_bytes()).get("CFBundleIdentifier") != identifier:
            raise ValueError(f"refusing to overwrite unrelated bundle: {bundle}")
    executable = contents / "MacOS" / source.name
    executable.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, executable)
    info_path.write_bytes(plistlib.dumps({
        "CFBundleIdentifier": identifier,
        "CFBundleExecutable": source.name,
        "CFBundleName": source.name,
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
        "LSBackgroundOnly": True,
    }))
    return bundle, executable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--identity", required=True, help="codesign identity name or SHA-1")
    parser.add_argument("--team-id", required=True, help="Apple signing team / application prefix")
    parser.add_argument("--profile", required=True, type=pathlib.Path,
                        help="Apple-issued macOS .provisionprofile authorizing this identity, app ID and keychain group")
    parser.add_argument("--identifier", default="app.seal.secure", help="bundle identifier for CLI artifacts")
    parser.add_argument("paths", nargs="+", type=pathlib.Path, help="built .app bundles or CLI executables")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Z0-9]{10}", args.team_id):
        parser.error("--team-id must be a 10-character Apple team identifier")
    if not args.profile.is_file():
        parser.error("--profile must point to an Apple-issued macOS provisioning profile")
    for path in args.paths:
        if not path.exists():
            parser.error(f"build artifact does not exist: {path}")
    decoded = subprocess.check_output(["security", "cms", "-D", "-i", str(args.profile)])
    profile = plistlib.loads(decoded)
    cert_hash = certificate_hash(args.identity)
    prepared = []
    try:
        # Validate every artifact before changing any signature or bundle.
        for path in args.paths:
            identifier = args.identifier
            if path.suffix == ".app":
                identifier = plistlib.loads((path / "Contents/Info.plist").read_bytes())["CFBundleIdentifier"]
            entitlements = validate_profile(profile, args.team_id, identifier, cert_hash)
            prepared.append((path, identifier, entitlements))
    except ValueError as e:
        parser.error(str(e))
    with tempfile.TemporaryDirectory(prefix="seal-sign-") as tmp:
        entitlement_path = pathlib.Path(tmp) / "keychain.plist"
        for path, identifier, entitlements in prepared:
            if path.suffix == ".app":
                bundle = path
                info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
                executable = bundle / "Contents/MacOS" / info["CFBundleExecutable"]
            else:
                bundle, executable = cli_bundle(path, identifier)
            shutil.copy2(args.profile, bundle / "Contents/embedded.provisionprofile")
            entitlement_path.write_bytes(plistlib.dumps(entitlements))
            subprocess.run([
                "codesign", "--force", "--sign", args.identity,
                "--identifier", identifier, "--entitlements", str(entitlement_path), str(bundle),
            ], check=True)
            subprocess.run(["codesign", "--verify", "--strict", str(bundle)], check=True)
            print(f"Provisioned executable: {executable}")


if __name__ == "__main__":
    main()
