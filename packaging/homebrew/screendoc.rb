class Screendoc < Formula
  desc "Record your terminal and publish it as GIF, MP4, asciinema cast, and web player"
  homepage "https://github.com/GlobalTechInfo/screendoc"
  # Keep this in sync with the latest release. The release
  # workflow backfills this file on main after every tagged
  # release, so the formula on main is always one release
  # behind the newest tag.
  version "0.4.3"

  livecheck do
    url "https://api.github.com/repos/GlobalTechInfo/screendoc/releases/latest"
    regex(/v?([0-9]+\.[0-9]+\.[0-9]+)/i)
  end

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-apple-darwin.tar.gz"
      sha256 "3c25600c9849321b4721dcf8393ee8d8b0d78f3130e26dda3d0b6c95fedff629"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-apple-darwin.tar.gz"
      sha256 "b696fb3dec4c796886115687394f726505cf10c890c5a7a4d1cf209c950e0177"
    end
  end

  on_linux do
    if Hardware::CPU.aarch64?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "46fe500e052aacde6f0dab3e7633f2df4f122db725c10f4435f50578610d0fa9"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "7eff6c25b0158029c8d9d2094f7d36abe028692e43a7f72bd53eb2cb1945a753"
    end
  end

  def install
    bin.install "screendoc"
  end

  test do
    system "#{bin}/screendoc", "--version"
  end
end
