class Screendoc < Formula
  desc "Record your terminal and publish it as GIF, MP4, asciinema cast, and web player"
  homepage "https://github.com/GlobalTechInfo/screendoc"
  # Keep this in sync with the latest release. The release workflow's
  # "Update homebrew formula" step backfills the checksums below after
  # every tagged release (the formula on main is always one release
  # behind the newest tag; that is the intended invariant).
  version "0.4.1"

  livecheck do
    url "https://api.github.com/repos/GlobalTechInfo/screendoc/releases/latest"
    regex(/v?([\d]+\.[\d]+\.[\d]+)/i)
  end

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-apple-darwin.tar.gz"
      sha256 "ea59fb9375d35c17d7550aad11c292f9b3e95fb17e6f43e83bf1f7275403c9e7"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-apple-darwin.tar.gz"
      sha256 "72847638e5bb3a84732ceea3aab9968a60148617d59f66ffa12494729c4bdc2f"
    end
  end

  on_linux do
    if Hardware::CPU.aarch64?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "dba309d0224ef0145937e52cc30d445d002ea335ba44b052c5a492f485f9017d"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fab937e8ac1a98d7b2a4414dc1f0a997ce87f3d3e064d4fd8cecf26c7076bc60"
    end
  end

  def install
    bin.install "screendoc"
  end

  test do
    system "#{bin}/screendoc", "--version"
  end
end
