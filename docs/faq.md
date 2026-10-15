# FAQ

[Türkçe sorular aşağıda](#sss)

### Is it safe?

ReClaim only deletes what you confirmed in a preview. Items it is not sure about are marked "Review first" and are not preselected; risky ones stay hidden. System folders, your profile and libraries, OneDrive, `.git` folders and anything you exclude in Settings can never be removed. Links are never followed. See [the safety model](architecture.md#safety-model).

### Why are caches deleted permanently instead of going to the Recycle Bin?

Caches rebuild themselves, and moving gigabytes of cache into the Recycle Bin frees nothing until you empty it. Your own files (large files, duplicates, empty folders, old installers) go to the Recycle Bin by default. Both choices are in Settings and every confirm dialog says which one is used.

### Why does it ask for administrator rights?

Only for items under Windows-owned folders, such as `C:\Windows\Temp` or the Windows Update download cache. The main app stays a normal user process; one UAC prompt starts a short-lived helper for those items. You can also restart the whole app as administrator from Settings.

### The numbers do not match Explorer.

ReClaim counts file sizes, not allocated clusters, and skips online-only OneDrive files because they take no local space. Folders you cannot read are listed as "Not scanned". Hard-linked files (common in `C:\Windows\WinSxS`) can be counted more than once, as Explorer also does.

### Why is my browser cache marked "Close Chrome to clean"?

Browsers keep their cache files open. Cleaning while the browser runs would fail halfway or corrupt the cache, so the rule waits until the browser is closed.

### Does it clean the registry?

No. Microsoft does not support registry cleaners and the space gain is negligible.

### Does it send any data?

No. There is no telemetry or account. The optional update check sends one request to the GitHub API.

### Where are settings and history stored?

`%APPDATA%\ReClaim`. In the portable build, in the `data` folder next to the exe.

### How do I add a location ReClaim does not know?

Open a "Cleanup rule request" issue, or write a rule yourself: [docs/rules.md](rules.md).

---

## SSS

### Güvenli mi?

ReClaim sadece önizlemede onayladığın şeyleri siler. Emin olmadığı öğeler "Önce gözden geçir" olarak işaretlenir ve önceden seçilmez; riskli olanlar gizli kalır. Sistem klasörleri, kullanıcı ve kitaplık klasörlerin, OneDrive, `.git` klasörleri ve Ayarlar'da hariç tuttuğun her şey hiçbir şekilde silinemez. Bağlantılar asla izlenmez.

### Önbellekler neden Geri Dönüşüm Kutusu'na değil de kalıcı siliniyor?

Önbellekler kendini yeniden kurar; gigabaytlarca önbelleği Geri Dönüşüm Kutusu'na taşımak, kutuyu boşaltana kadar alan açmaz. Senin dosyaların (büyük dosyalar, kopyalar, boş klasörler, eski kurulum dosyaları) varsayılan olarak Geri Dönüşüm Kutusu'na gider. İkisi de Ayarlar'dan değiştirilebilir.

### Neden yönetici izni istiyor?

Sadece `C:\Windows\Temp` ya da Windows Update indirme önbelleği gibi Windows'a ait klasörlerdeki öğeler için. Ana uygulama normal kullanıcı olarak kalır; tek bir UAC onayı bu öğeler için kısa ömürlü bir yardımcı başlatır.

### Sayılar Gezgin'le tutmuyor.

ReClaim dosya boyutlarını sayar, sadece bulutta duran OneDrive dosyalarını yerel alan kaplamadıkları için atlar. Okunamayan klasörler "Taranmadı" olarak gösterilir.

### Kayıt defterini temizliyor mu?

Hayır. Microsoft kayıt defteri temizleyicileri desteklemez, kazanılan alan da önemsizdir.

### Veri gönderiyor mu?

Hayır. Telemetri ve hesap yok. İsteğe bağlı güncelleme denetimi GitHub API'ye tek bir istek atar.

### Ayarlar ve geçmiş nerede duruyor?

`%APPDATA%\ReClaim`. Taşınabilir sürümde exe'nin yanındaki `data` klasöründe.
