//! "Iniciar com o Windows": grava o agente em
//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run, só para o usuário atual,
//! sem pedir permissão de administrador. Ele abre minimizado.

#[cfg(windows)]
mod imp {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    const CHAVE: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NOME: &str = "BalancaAgent";

    pub fn ativo() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(CHAVE, KEY_READ)
            .and_then(|k| k.get_value::<String, _>(NOME))
            .is_ok()
    }

    pub fn definir(ativar: bool) -> Result<(), String> {
        let chave = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(CHAVE, KEY_SET_VALUE)
            .map_err(|e| format!("Não foi possível abrir o registro: {e}"))?;
        if ativar {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            chave
                .set_value(NOME, &format!("\"{}\" --minimizado", exe.display()))
                .map_err(|e| format!("Não foi possível gravar no registro: {e}"))
        } else {
            match chave.delete_value(NOME) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("Não foi possível remover do registro: {e}")),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn ativo() -> bool {
        false
    }

    pub fn definir(ativar: bool) -> Result<(), String> {
        if ativar {
            Err("Iniciar com o sistema só está disponível no Windows.".into())
        } else {
            Ok(())
        }
    }
}

pub use imp::{ativo, definir};
