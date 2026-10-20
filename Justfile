set shell := ["bash", "-uc"]

# Varsayılan: Doğrulama protokolünü çalıştır
default: verify

# Ajan ve Sistem Doğrulama Protokolü
verify: check-leaks
	@echo "✅ Thornhill statik doğrulamaları başarıyla geçti!"

# 1. Sabit kullanıcı yolları (/home/...) ve sızıntı denetimi
check-leaks:
	@echo "🔍 Sabit yol ve sızıntı denetimi yapılıyor..."
	@! grep -rn --exclude-dir=".git" --exclude="*.md" --exclude="Justfile" "/home/" . 2>/dev/null || (echo "❌ HATA: Kod içinde sabit /home/ yolu tespit edildi!" && exit 1)
	@echo "✅ Sabit yol sızıntısı yok."

# Geliştirme ortamı (Nix Shell)
dev:
	nix develop -c $$SHELL

# Format kontrolü
fmt:
	@if command -v cargo >/dev/null 2>&1; then \
		cargo fmt --all; \
	fi
	nix fmt 2>/dev/null || true
