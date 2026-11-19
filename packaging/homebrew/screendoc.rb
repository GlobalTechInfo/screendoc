class Screendoc < Formula
  desc "Record your terminal and publish it as GIF, MP4, asciinema cast, and web player"
  homepage "https://github.com/GlobalTechInfo/screendoc"
  # Keep this in sync with the latest release. The release
  # workflow backfills this file on main after every tagged
  # release, so the formula on main is always one release
  # behind the newest tag.
  version "0.4.2"

  livecheck do
    url "https://api.github.com/repos/GlobalTechInfo/screendoc/releases/latest"
    regex(/v?([0-9]+\.[0-9]+\.[0-9]+)/i)
  end

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-apple-darwin.tar.gz"
      sha256 "26f55adbde98eb6071828da836cd515fa1ae1362a4e072ad8f73233612fa289c"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-apple-darwin.tar.gz"
      sha256 "319dc54f21b384397c3cde2585068bda8fb74edd6fe0040ca9f4271caf9cb519"
    end
  end

  on_linux do
    if Hardware::CPU.aarch64?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0ce11d1fdc0d1da91cce63f61ccba65be8766f2ac480c1f22530c6a15a03bb76"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fc7b74f9f6a2d67ba0afe3ce219359fd366245837d9a99d66e32146fe31417a1"
    end
  end

  def install
    bin.install "screendoc"
  end

  test do
    system "#{bin}/screendoc", "--version"
  end
end
