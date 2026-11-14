#!/usr/bin/env python3
"""Real public-Tor interop. ADB carries only public invitations and test progress, never messages/ACKs."""
from pathlib import Path
import argparse,os,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--serial',default='emulator-5554');p.add_argument('--tor',required=True);p.add_argument('--root',required=True);p.add_argument('--cache');p.add_argument('--groups',action='store_true');p.add_argument('--apk',default='dist/psychotalk-0.3.1-experimental-arm64.apk');a=p.parse_args()
os.chdir(Path(__file__).resolve().parent.parent)
base=Path(a.root).resolve();assert not base.exists(),'Use a fresh disposable fixture directory';base.mkdir(parents=True,mode=0o700)
adb=['adb','-s',a.serial]
for apk in [a.apk,'android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk']:subprocess.run(adb+['install','-r',apk],check=True)
# A cancelled prior test can leave this file: never pair with that old identity.
subprocess.run(adb+['shell','run-as','app.psychotalk','rm','-f','no_backup/psychotalk-tests/android.code'],check=True)
cache='/data/user/0/app.psychotalk/files/interop-tor-cache'
if a.cache:
 subprocess.run(adb+['shell','run-as','app.psychotalk','mkdir','-p','files/interop-tor-cache'],check=True)
 for f in Path(a.cache).glob('cached-*'):
  if f.is_file() and f.name in ['cached-certs','cached-microdesc-consensus','cached-microdescs','cached-microdescs.new']:
   subprocess.run(adb+['shell',f"run-as app.psychotalk sh -c 'cat > files/interop-tor-cache/{f.name}'"],input=f.read_bytes(),check=True)
command=['target/debug/examples/android_probe',a.tor,str(base)]
if a.cache or a.groups:command.append(a.cache or '-')
if a.groups:command.append('--groups')
with (base/'host.log').open('w') as host_log,(base/'android.log').open('w') as android_log:
 host=subprocess.Popen(command,stdout=host_log,stderr=subprocess.STDOUT);android=None
 try:
  end=time.monotonic()+60
  while not (base/'desktop.code').exists():
   if host.poll() is not None or time.monotonic()>end:raise RuntimeError('Desktop fixture did not start; inspect host.log')
   time.sleep(.2)
  args=['shell','am','instrument','-w','-e','class','app.psychotalk.DesktopInteropTest','-e','desktopCode',(base/'desktop.code').read_text()]
  if a.cache:args+=['-e','cacheDir',cache]
  if a.groups:args+=['-e','groupTest','1']
  android=subprocess.Popen(adb+args+['app.psychotalk.test/androidx.test.runner.AndroidJUnitRunner'],stdout=android_log,stderr=subprocess.STDOUT)
  end=time.monotonic()+60
  while time.monotonic()<end:
   v=subprocess.run(adb+['shell','run-as','app.psychotalk','cat','no_backup/psychotalk-tests/android.code'],capture_output=True,text=True)
   if v.returncode==0 and v.stdout.strip().startswith('psychotalk:1:'):(base/'android.code').write_text(v.stdout.strip());break
   if android.poll() is not None:raise RuntimeError('Android stopped before emitting its public code')
   time.sleep(.25)
  else:raise RuntimeError('Android public code timeout')
  end=time.monotonic()+2400;relayed=set()
  while android.poll() is None:
   for name in ['desktop.exchange','desktop.confirmed','group.invite','group.exchange','group.confirmed','group.reopen','group.final.confirmed']:
    if name not in relayed and (base/name).exists():
     subprocess.run(adb+['shell',f"run-as app.psychotalk sh -c 'cat > no_backup/psychotalk-tests/{name}'"],input=(base/name).read_bytes(),check=True);relayed.add(name)
   if a.groups and not (base/'android.offline').exists():
    offline=subprocess.run(adb+['shell','run-as','app.psychotalk','test','-f','no_backup/psychotalk-tests/android.offline'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    if offline.returncode==0:(base/'android.offline').write_text('ok')
   if host.poll() is not None and host.returncode!=0:raise RuntimeError('Desktop probe failed; inspect host.log')
   if time.monotonic()>end:raise RuntimeError('Interop timed out')
   time.sleep(.25)
  host.wait(timeout=60)
  android_log.flush();assert 'OK (1 test)' in (base/'android.log').read_text(),'Android test failed';assert host.returncode==0,'Desktop probe failed'
  print('PASS: actual desktop/Android Tor interoperability'+(' and mixed group/offline per-recipient deletion' if a.groups else ''))
 finally:
  if android is not None and android.poll() is None:subprocess.run(adb+['shell','am','force-stop','app.psychotalk'])
  if host.poll() is None:host.terminate();host.wait(timeout=15)
