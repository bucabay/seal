cask "seal" do
  version "0.2.1"

  on_arm do
    sha256 "118ad965741cda862e5c7b42ea7d2ee5f4b189a5b6e001d04b84bdfdb6ad03f1"
    url "https://github.com/bucabay/seal/releases/download/v#{version}/Seal_#{version}_aarch64.dmg"
  end

  on_intel do
    sha256 "2ddc1c1f27acc2b466a7a5aec9c51f309c343e2841c529046d74e92f52a045bb"
    url "https://github.com/bucabay/seal/releases/download/v#{version}/Seal_#{version}_x64.dmg"
  end

  name "Seal"
  desc "Secrets manager backed by the OS keychain"
  homepage "https://github.com/bucabay/seal"

  app "Seal.app"

  caveats <<~EOS
    Seal is distributed unsigned and not notarized, so macOS blocks the first
    launch. Remove the quarantine flag once:
      xattr -dr com.apple.quarantine /Applications/Seal.app
    or install without it:
      brew install --cask --no-quarantine seal
    On macOS 15 and later, allow the app under System Settings → Privacy &
    Security after the first blocked launch.
  EOS
end
