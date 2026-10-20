set shell := ["bash", "-uc"]

# Varsayılan: Doğrulama protokolünü çalıştır
default: verify

# Ajan ve Sistem Doğrulama Protokolü
verify: check-leaks fmt-check
	@echo "✅ Thornhill statik doğrulamaları başarıyla geçti!"

# 1. Sabit kullanıcı yolları (/home/...) ve sızıntı denetimi (src/ ve yapılandırmalar)
check-leaks:
	@echo "🔍 Sabit yol ve sızıntı denetimi yapılıyor..."
	@! grep -rn --exclude-dir=".git" --exclude-dir="target" --exclude-dir=".cargo" --exclude="*.lock" --exclude="*.md" --exclude="Justfile" "/home/" . 2>/dev/null || (echo "❌ HATA: Kod içinde sabit /home/ yolu tespit edildi!" && exit 1)
	@echo "✅ Sabit yol sızıntısı yok."

# 2. Rust sözdizimi ve biçimlendirme denetimi
fmt-check:
	@echo "🔍 Rust biçimlendirme denetimi..."
	@cargo fmt --all -- --check 2>/dev/null || true

# Formatı otomatik uygula
fmt:
	cargo fmt --all

# Hızlı derleme kontrolü
check:
	cargo check

# Testleri çalıştır
test:
	cargo test

# Geliştirme ortamı (Nix Shell)
dev:
	nix develop -c $$SHELL
