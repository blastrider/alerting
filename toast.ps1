param(
    [string]$Summary,
    [string]$Body
)

Add-Type -AssemblyName System.Runtime.WindowsRuntime | Out-Null
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom, ContentType=WindowsRuntime] | Out-Null

$xml = "<toast><visual><binding template='ToastGeneric'><text>$Summary</text><text>$Body</text></binding></visual></toast>"
$doc = [Windows.Data.Xml.Dom.XmlDocument]::new()
$doc.LoadXml($xml)
$toast = [Windows.UI.Notifications.ToastNotification]::new($doc)
$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('WindowsPowerShell')
$notifier.Show($toast) | Out-Null
