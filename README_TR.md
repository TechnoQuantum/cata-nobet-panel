# Nöbetçi Eczane Paneli — CATA CT-4568

Bu Windows uygulaması nöbet listesini Bluetooth ile 96 × 16 LED panele gönderir. Liste elle girilebilir, satır satır yapıştırılabilir ya da aylık Excel/ZIP dosyasından içe aktarılabilir. Uygulama o vardiyanın nöbetçi eczanelerini her gün kendiliğinden panele gönderir. Bilgisayar yeniden başlasa ya da elektrik kesilse de bu gönderim devam eder.

## Kurulum

1. [Sürümler](https://github.com/TechnoQuantum/cata-nobet-panel/releases) sayfasından `NobetPanel-<sürüm>.zip` dosyasını indirip bir klasöre açın.
2. **Kur.cmd** dosyasına çift tıklayın ve yönetici izni isteğini onaylayın.

Kurulum şunları yapar:

- Programı `C:\Program Files\NobetPanel` klasörüne kopyalar.
- 1.x sürümünün verilerini `%LOCALAPPDATA%\NobetPanel` konumundan `C:\ProgramData\NobetPanel` konumuna taşır.
- **NobetPanel** Windows servisini kurar. Servis Windows açılınca başlar ve oturum açılmasa da paneli günceller. Beklenmedik biçimde durursa kendini yeniden başlatır.
- Oturum açılınca pencereyi simge durumunda başlatır, Başlat menüsüne ve masaüstüne kısayol ekler.
- Eski 1.x başlangıç kaydını kaldırır.

Kaldırmak için kurulum klasöründeki `uninstall.ps1` dosyasını çalıştırın. Bu, nöbet listesini ve ayarları korur. Bunları da silmek için `-RemoveData` ekleyin.

## Nasıl çalışır

- **Servis ve pencere:** Otomatik gönderimi servis yapar. Pencere listeyi düzenlemek, görünümü ayarlamak ve elle gönderim için kullanılır. İkisi aynı veri dosyasını kullanır ve birbirinin değişikliğini birkaç saniye içinde görür. Panele aynı anda yalnızca biri bağlanır.
- **Yeniden başlamaya dayanıklı:** İçerik paneldeki bir program yuvasına kaydedilir (varsayılan: slot 1) ve panelin açılış programı olarak seçilir. 1.x sürümü logolu yazıyı geçici canlı ekrana gönderiyordu. Bu yüzden panel yeniden başlayınca varsayılan "LED" ekranına dönüyordu.
- **Teslim onayı:** Her veri parçası ve her komut için panelin onayı beklenir. Gönderim, panel onaylamadan başarılı sayılmaz. Başarısız gönderim üç kez denenir. Otomatik denemeler arasındaki bekleme 30 saniyeden başlayıp 10 dakikaya kadar uzar.
- **Değişince yeniden gönderir:** Bugünün kaydı ya da görünüm ayarlarından biri değişince panel kendiliğinden güncellenir. Bu ayarlar logo, renk, hız, efekt, yazı tipi, ayna ve parlaklıktır.
- **Vardiya saatleri:** Normal nöbet 18:00'de başlar, ertesi sabah 08:30'da biter. Pazar nöbeti Pazar 08:30 ile Pazartesi 08:30 arasıdır. Nöbet tarihi her sabah 08:30'da değişir.
- **Onaylar:** Göndermeden önce önizleme gösterilir (bu adım kapatılabilir). Kayıt silme, içe aktarmada var olan kaydı değiştirme, panel belleğini temizleme ve paneli kapatma için önce onay istenir.

## Nöbet listesi

Toplu yapıştırma biçimi:

```text
2026-10-08 | Örnek Eczanesi | Merkez Eczanesi
09.10.2026 | Yeni Eczane
```

Ayraç olarak sekme, `|`, `;` veya `,` kullanılabilir. Tarihten sonraki her sütun bir eczane adıdır. Kabul edilen tarih biçimleri: `YYYY-AA-GG`, `GG.AA.YYYY`, `GG/AA/YYYY`, `GG-AA-YYYY`. İçe aktarmadan önce kaç kaydın yeni, kaçının değişen olduğu gösterilir.

**Aylık Excel / ZIP** seçeneği eczacı odasının aylık `.xlsx` listesini ya da bu listeleri içeren `.zip` arşivini okur. Yıl ve ay başlıktan ya da dosya adından alınır.

## Görünüm

- Yazı ve arka plan rengi, efekt, hız, parlaklık, TTF/OTF yazı tipi seçilebilir. Hazır yazı tipi (Unifont) Türkçe karakterleri içerir.
- Kayan listenin yanında, isteğe bağlı olarak sabit, çerçevesi yanıp sönen kırmızı **E** eczane logosu gösterilebilir.
- **BUGÜN NÖBETÇİYİZ:** Kendi eczanenizin adını girin. O gün nöbetçi sizseniz bu mesaj logoyla birlikte gösterilir.
- **Ayna / yön:** Panel ters takılıysa yatay ve dikey ayna ayarlarıyla düzeltilir.

## Gereksinimler ve ipuçları

- Windows 10/11 ve Bluetooth LE desteği.
- Panel açık ve menzilde olmalı. Sinyal −80 dBm'den zayıfsa bağlantı ara sıra kopabilir. Bu durumda bilgisayarı yaklaştırın ya da USB Bluetooth adaptörü kullanın.
- Telefondaki iPixel Color uygulamasını kapatın. Panel aynı anda yalnızca bir Bluetooth bağlantısı kabul eder.
- Kayıt dosyası: `C:\ProgramData\NobetPanel\nobet_panel.log`. Uygulamadaki **Günlük** sayfasında da görünür.

Bu uygulama CATA, Heaton veya iPIXEL'in resmî ürünü değildir.
