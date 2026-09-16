//! Credential boundary: native storage or an explicitly selected public fixture.
use super::*;

#[derive(Clone)]
pub(super) enum Credentials {
    Native,
    #[cfg(feature = "ui-test-support")]
    Fixture(PathBuf),
}
impl Credentials {
    pub(super) fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        match self {
            Self::Native => native::get(service, account),
            #[cfg(feature = "ui-test-support")]
            Self::Fixture(directory) => {
                let path = directory
                    .join(Digest::of(format!("{service}:{account}").as_bytes()).to_string());
                match File::open(path) {
                    Ok(file) => {
                        let mut bytes = Zeroizing::new(Vec::new());
                        file.take(65537)
                            .read_to_end(&mut bytes)
                            .map_err(|_| ProfileError::Io)?;
                        if bytes.len() > 65536 {
                            return Err(ProfileError::Invalid);
                        }
                        Ok(Some(bytes))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(_) => Err(ProfileError::Io),
                }
            }
        }
    }
    pub(super) fn set(
        &self,
        service: &str,
        account: &str,
        bytes: &[u8],
    ) -> Result<(), ProfileError> {
        match self {
            Self::Native => native::set(service, account, bytes),
            #[cfg(feature = "ui-test-support")]
            Self::Fixture(directory) => {
                std::fs::create_dir_all(directory).map_err(|_| ProfileError::Io)?;
                let path = directory
                    .join(Digest::of(format!("{service}:{account}").as_bytes()).to_string());
                let mut temp =
                    tempfile::NamedTempFile::new_in(directory).map_err(|_| ProfileError::Io)?;
                temp.write_all(bytes).map_err(|_| ProfileError::Io)?;
                temp.as_file().sync_all().map_err(|_| ProfileError::Io)?;
                temp.persist(path).map_err(|_| ProfileError::Io)?;
                File::open(directory)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| ProfileError::Io)
            }
        }
    }
}

#[cfg(all(test, feature = "ui-test-support"))]
mod tests {
    use super::*;

    #[test]
    fn fixture_credentials_survive_independent_loads_and_keep_profiles_separate() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let lease = NativeProfile::acquire_test(first.path()).unwrap();
        let author = lease.profile().author().unwrap().public();
        lease.profile().save_state("PUBLIC-state", &42_u64).unwrap();
        let loaded = NativeProfile::load_test(first.path()).unwrap();
        assert_eq!(loaded.author().unwrap().public(), author);
        assert_eq!(loaded.load_state::<u64>("PUBLIC-state").unwrap(), Some(42));
        assert!(matches!(
            NativeProfile::acquire_test(first.path()),
            Err(ProfileError::Busy)
        ));
        let other = NativeProfile::acquire_test(second.path()).unwrap();
        assert_ne!(
            other.profile().transport_public(),
            loaded.transport_public()
        );
        assert_eq!(
            other.profile().load_state::<u64>("PUBLIC-state").unwrap(),
            None
        );
        drop(lease);
        assert!(NativeProfile::acquire_test(first.path()).is_ok());
    }

    #[test]
    fn corrupt_fixture_state_is_an_error_without_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Credentials::Fixture(directory.path().to_owned());
        credentials
            .set("PUBLIC-service", "PUBLIC-account", b"PUBLIC")
            .unwrap();
        let path = directory
            .path()
            .join(Digest::of(b"PUBLIC-service:PUBLIC-account").to_string());
        std::fs::write(&path, vec![0; 65537]).unwrap();
        assert!(matches!(
            credentials.get("PUBLIC-service", "PUBLIC-account"),
            Err(ProfileError::Invalid)
        ));
        assert_eq!(std::fs::metadata(path).unwrap().len(), 65537);
    }
}
