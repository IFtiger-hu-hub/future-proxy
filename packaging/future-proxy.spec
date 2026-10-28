%global debug_package %{nil}
%global __strip /bin/true
%undefine _missing_build_ids_terminate_build

Name:           future-proxy
Version:        1.0.0
Release:        1%{?dist}
Summary:        Ultra-lightweight Native GUI Proxy Client (Future Proxy)
License:        GPL-3.0
URL:            https://github.com/MetaCubeX/mihomo

Source0:        future-proxy
Source1:        mihomo
Source2:        future-proxy.desktop
Source3:        future-proxy.svg
Source4:        config.yaml
Source5:        country.mmdb
Source6:        geosite.dat

BuildArch:      x86_64
Requires:       glibc, libcap-progs, curl

%description
Future Proxy is an ultra-lightweight standalone native GUI proxy client written in Rust.
Zero external runtime dependencies. Stores all user profiles safely in ~/.config/future-proxy/.
Memory footprint is only ~50MB.

%install
mkdir -p %{buildroot}%{_bindir}
mkdir -p %{buildroot}%{_datadir}/applications
mkdir -p %{buildroot}%{_datadir}/icons/hicolor/scalable/apps
mkdir -p %{buildroot}%{_datadir}/future-proxy

install -m 755 %{SOURCE0} %{buildroot}%{_bindir}/future-proxy
install -m 755 %{SOURCE1} %{buildroot}%{_bindir}/future-proxy-core

install -m 644 %{SOURCE2} %{buildroot}%{_datadir}/applications/future-proxy.desktop
install -m 644 %{SOURCE3} %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/future-proxy.svg

install -m 644 %{SOURCE4} %{buildroot}%{_datadir}/future-proxy/default-config.yaml
install -m 644 %{SOURCE5} %{buildroot}%{_datadir}/future-proxy/country.mmdb
install -m 644 %{SOURCE6} %{buildroot}%{_datadir}/future-proxy/geosite.dat

%post
setcap 'cap_net_admin,cap_net_bind_service=+ep' %{_bindir}/future-proxy-core 2>/dev/null || true

if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database &>/dev/null || :
fi

echo ""
echo "=========================================================="
echo "  🎉 Future Proxy 客户端安装完成！"
echo "  - 启动命令：future-proxy"
echo "  - 菜单快捷方式：Future Proxy"
echo "=========================================================="
echo ""

%postun
if [ -x %{_bindir}/gtk-update-icon-cache ]; then
    %{_bindir}/gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :
fi
if [ -x %{_bindir}/update-desktop-database ]; then
    %{_bindir}/update-desktop-database &>/dev/null || :
fi

%files
%{_bindir}/future-proxy
%{_bindir}/future-proxy-core
%{_datadir}/applications/future-proxy.desktop
%{_datadir}/icons/hicolor/scalable/apps/future-proxy.svg
%{_datadir}/future-proxy/
