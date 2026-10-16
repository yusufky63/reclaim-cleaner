# ReClaim Cleaner

Windows 10 ve 11 için açık kaynak disk temizleyici ve alan analizcisi. Alanının nereye gittiğini gösterir, bulduğu her şeyi silinme güvenliğine göre sıralar ve sen gözden geçirmeden hiçbir şeyi silmez.

[Son sürümü indir](https://github.com/yusufky63/reclaim-cleaner/releases/latest) · [English](README.md) · [Neyin güvenli olduğuna nasıl karar verir](docs/architecture.md#safety-model)

![Genel bakış](docs/screenshots/overview-light.png)

## Neler yapar

- **Genel bakış.** Şu anki ve temizlik sonrası boş alan; "Silinmesi güvenli" ve "Gözden geçirilecek" olarak ayrılmış tek bir öneri listesi. Seçtiklerini hemen temizleyebilir ya da PowerShell betiği olarak alabilirsin.
- **Alan haritası.** Herhangi bir sürücünün ya da klasörün treemap görünümü; projeler, medya, önbellekler, uygulamalar, yapay zeka modelleri ve Windows olarak renklenir. Temizlik görünümü sadece geri kazanılabilir alanı boyar.
- **Gereksiz dosyalar.** Bilinen 118 konum: Windows geçici dosyaları ve kayıtları, çökme dökümleri, tarayıcı önbellekleri (Chrome, Edge, Brave, Firefox ve diğerleri), uygulama önbellekleri (Discord, Slack, Teams, VS Code, Spotify), oyun başlatıcıları ve geliştirici önbellekleri (npm, pnpm, pip, uv, Cargo, Gradle, NuGet, Go, Playwright).
- **Geliştirici.** Projelerindeki `node_modules`, `target`, `.next`, `__pycache__`, `.venv` ve 40 başka derleme klasörünü bulur; sadece 30 gündür dokunmadığın projeleri önceden seçer.
- **Büyük dosyalar, kopyalar, boş klasörler.** Tür ve yaşa göre süzülür; kopyalar tam BLAKE3 özetiyle doğrulanır, sabit bağlantılar bir kez sayılır.
- **Uygulamalar ve başlangıç.** Masaüstü ve Store uygulamalarını kendi kaldırıcılarıyla kaldırır, ardından AppData, ProgramData ve Program Files'taki artık klasörleri güven düzeyiyle listeler; başlangıç uygulamalarını Görev Yöneticisi'nin yaptığı gibi geri alınabilir şekilde kapatır.
- **Windows araçları.** Geri Dönüşüm Kutusu, hazırda bekleme dosyası, bileşen deposu temizliği (DISM), Teslim İyileştirme ve önceki Windows kurulumu; hepsi Windows'un kendi komutlarıyla.
- **Türkçe ve İngilizce**, açık ve koyu tema, taşınabilir mod, yerel temizlik geçmişi.

## İndir

| Dosya | Kimin için |
|---|---|
| `ReClaim-Cleaner_<sürüm>_x64-setup.exe` | Önerilen. Geçerli kullanıcıya kurulur, yönetici izni istemez, kurulum Türkçe ya da İngilizce. |
| `ReClaim-Cleaner_<sürüm>_x64_en-US.msi` / `_tr-TR.msi` | Kurumsal dağıtım ve Grup İlkesi. |
| `ReClaim-Cleaner_<sürüm>_x64_portable.zip` | Kurulumsuz. Ayarlar ve geçmiş exe'nin yanındaki `data` klasöründe durur. Windows 10 ve 11'de zaten bulunan WebView2 gerekir. |

Her sürümde SHA-256 özetleri yayımlanır. Derlemeler henüz kod imzalı değil; SmartScreen "Windows bilgisayarınızı korudu" diyebilir. **Ek bilgi**'ye tıkla, dosya adının sürümle eşleştiğini kontrol et, sonra **Yine de çalıştır**.

## Güvenlik modeli

1. **Önce önizleme.** Taramalar hiçbir şey silmez. Her temizlik, boyutunu ve riskini gördüğün bir listeden yapılır.
2. **Üç risk düzeyi.** Güvenli öğeler (kendini yeniden kuran önbellekler) önceden seçilir. Gözden geçirilecekler görünür ama seçili gelmez. Dikkat öğeleri Ayarlar'da açana kadar gizlidir.
3. **Korunan konumlar.** Sürücü kökleri, Windows sistem klasörleri, Program Files, kullanıcı ve kitaplık klasörlerin, OneDrive, `.git` klasörleri ve uygulamanın kendi verisi hiçbir şekilde silinemez. Hariç tuttuğun klasörler her yerde geçerlidir.
4. **Bağlantılar asla izlenmez.** Junction ve sembolik bağlantılar bağlantı olarak silinir ya da atlanır. Sadece bulutta duran dosyalar açılmaz, indirilmez.
5. **Açık uygulamalar önbelleklerini korur.** Chrome açıkken önbelleğine dokunulmaz. Süreçler asla sonlandırılmaz.
6. **İşe uygun yöntem.** Önbellekler varsayılan olarak kalıcı silinir, çünkü 10 GB önbelleği Geri Dönüşüm Kutusu'na taşımak alan açmaz. Senin dosyaların varsayılan olarak Geri Dönüşüm Kutusu'na gider. İkisi de ayarlanabilir.
7. **En az yetki.** Uygulama normal kullanıcı olarak çalışır. Yönetici gerektiren öğeleri, UAC ile başlatılan aynı exe yardımcı modda temizler; bu mod sadece kural kimliklerini alır, yol almaz.
8. **Kayıt defteri temizleyici yok, ince ayar yok, telemetri yok.** Tek ağ isteği, isteğe bağlı GitHub güncelleme denetimidir.

## Kaynaktan derleme

Gerekenler: Windows 10 ya da 11, [Rust](https://rustup.rs) (stable, MSVC), Node.js 22+, pnpm 9 ve Visual Studio C++ derleme araçları.

```bash
pnpm install
pnpm app:dev
```

## Lisans

[MIT](LICENSE).
