# Read PE version resources or the MSI Property table. Never runs the installer.
$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
try {
    $spec=[Console]::In.ReadToEnd() | ConvertFrom-Json
    $result=@{name='';publisher='';version='';product_code=$null;architecture=$null}
    if($spec.extension -eq 'msi'){
        $installer=New-Object -ComObject WindowsInstaller.Installer
        $db=$installer.OpenDatabase([string]$spec.path,0)
        $summary=$db.GetType().InvokeMember('SummaryInformation',[Reflection.BindingFlags]::GetProperty,$null,$db,0)
        $template=[string]$summary.GetType().InvokeMember('Property',[Reflection.BindingFlags]::GetProperty,$null,$summary,7)
        $result.architecture=switch(($template -split ';')[0]){'Intel'{'x86'};'x64'{'x64'};'Arm64'{'arm64'};default{$null}}
        $view=$db.OpenView('SELECT `Property`,`Value` FROM `Property`')
        $view.Execute()
        try {
            while($row=$view.Fetch()){
                $name=[string]$row.GetType().InvokeMember('StringData',[Reflection.BindingFlags]::GetProperty,$null,$row,1)
                if($name -notin @('ProductName','Manufacturer','ProductVersion','ProductCode')){continue}
                $value=[string]$row.GetType().InvokeMember('StringData',[Reflection.BindingFlags]::GetProperty,$null,$row,2)
                switch($name){'ProductName'{$result.name=$value};'Manufacturer'{$result.publisher=$value};'ProductVersion'{$result.version=$value};'ProductCode'{$result.product_code=([Guid]$value).ToString()}}
            }
        } finally{$view.Close()}
    } elseif($spec.extension -eq 'exe'){
        $info=[Diagnostics.FileVersionInfo]::GetVersionInfo([string]$spec.path)
        $result.name=[string]$info.ProductName;$result.publisher=[string]$info.CompanyName;$result.version=[string]$info.ProductVersion
        $reader=[IO.BinaryReader]::new([IO.File]::OpenRead([string]$spec.path))
        try {
            $null=$reader.BaseStream.Seek(60,[IO.SeekOrigin]::Begin);$offset=$reader.ReadUInt32()
            if($offset -lt 64 -or $offset -gt 1048576){throw 'Invalid PE offset'}
            $null=$reader.BaseStream.Seek($offset,[IO.SeekOrigin]::Begin)
            if($reader.ReadUInt32() -ne 17744){throw 'Invalid PE signature'}
            $result.architecture=switch($reader.ReadUInt16()){332{'x86'};34404{'x64'};43620{'arm64'};default{$null}}
        } finally{$reader.Dispose()}
    } else {exit 1}
    [Console]::Out.Write(($result | ConvertTo-Json -Compress))
} catch {exit 1}
